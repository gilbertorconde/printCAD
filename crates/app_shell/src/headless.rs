//! Running a script without a window: `printcad --script build.lua`, for
//! batch exports, checks and tests.
//!
//! The script sees the workbenches' commands and the document's own
//! (reading it, adding and naming bodies, visibility, deleting, rebuilding
//! and measuring), and `file.import`, `file.save_as` and `file.export`
//! with a path, an import read and landed on this thread as the app lands
//! it (`step_import::land_import`). What
//! needs a window (the selection, the view, tools) answers that it is not
//! available. Rebuilds run on this thread, so a script can build, measure
//! and export in one go.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService,
    FeatureId, ParamKind, WorkbenchRuntimeContext,
};
use kernel_api::TessellationSettings;
use serde_json::{Value, json};

use crate::app::scripts::{doc_commands, document_command, tree_item};
use crate::ui::TreeItemId;

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
        Some(path) => {
            let mut document = Document::load_from_file(path)
                .with_context(|| format!("could not open {}", path.display()))?;
            // Solids are derived: build them all afresh.
            registry.invalidate_all(&mut document);
            document
        }
        None => Document::new("Untitled"),
    };
    let mut host = Headless {
        registry,
        document,
        file: invocation.open.clone(),
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
/// way `printcad --script` runs a file, and hand the registry back: the
/// document the script left, or the error that stopped it.
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
#[cfg(test)]
pub(crate) fn run_in_document(
    registry: &mut DocumentService,
    document: Document,
    source: &str,
    name: &str,
) -> (Document, Result<Option<Value>, String>) {
    let mut host = Headless {
        registry: std::mem::take(registry),
        document,
        file: None,
    };
    let out = scripting::ScriptEngine::new().run_script(source, name, &mut host);
    *registry = host.registry;
    let answer = match out.error {
        Some(error) => Err(error),
        None => Ok(out.returned),
    };
    (host.document, answer)
}

/// The document and the workbenches, with no window.
struct Headless {
    registry: DocumentService,
    document: Document,
    file: Option<PathBuf>,
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
    "doc.rename",
    "doc.set_visible",
    "doc.delete",
    "doc.suppress",
    "doc.move",
    "doc.set_tip",
    "doc.rebuild",
    "doc.faces",
    "doc.edges",
    "doc.set_body",
    "doc.set_face_color",
    "doc.set_textures",
    "doc.note_add",
    "doc.note_set",
    "doc.note_remove",
    "doc.notes",
    "doc.linked_copy",
    "doc.array",
    "doc.set_array",
    "doc.move_after",
    "doc.recompute",
    "doc.measure",
    "doc.print_layout",
    "doc.picture",
];

fn file_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(
            "file.import",
            "Read a STEP, IGES or mesh file, or a file a workbench reads, into the document",
        )
        .param("path", ParamKind::String, "The file to read"),
        CommandSpec::new("file.save_as", "Save the document").param(
            "path",
            ParamKind::String,
            "Where to save it",
        ),
        CommandSpec::new("file.export", "Write bodies as STEP, STL or 3MF")
            .param("path", ParamKind::String, "Where to write")
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
            .optional(
                "layout",
                ParamKind::Bool,
                "The print layout (doc.print_layout) rather than the bodies where they sit",
            )
            .returns("{path, written, skipped, triangles}"),
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
        let answer = self.call_one(id, args);
        self.settle_formulas();
        answer
    }
}

impl Headless {
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
        let spec = scripting::Host::commands(self)
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| match doc_commands().iter().any(|c| c.id == id) {
                true => CommandError::failed("not available without a window"),
                false => CommandError::unknown_among(
                    id,
                    scripting::Host::commands(self)
                        .iter()
                        .map(|c| c.id.as_str()),
                ),
            })?;
        spec.check(&args)?;
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
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                match tree_item(&self.document, a.id("id")?)? {
                    TreeItemId::Body(body) => self.document.set_body_visible(body, visible),
                    TreeItemId::Feature(feature) => {
                        self.document.set_feature_visible(feature, visible)
                    }
                    TreeItemId::Note(id) => {
                        if let Some((_, note)) = self.document.note(id) {
                            let mut note = note.clone();
                            note.hidden = !visible;
                            self.document.set_note(note);
                        }
                    }
                    _ => return Err(CommandError::bad("id", "cannot be hidden here")),
                }
                Ok(Value::Null)
            }
            "doc.delete" => {
                match tree_item(&self.document, a.id("id")?)? {
                    TreeItemId::Body(body) => {
                        self.document.remove_body(body);
                    }
                    TreeItemId::Feature(feature) => self.delete_feature(feature)?,
                    TreeItemId::Note(note) => {
                        self.document.remove_note(note);
                    }
                    _ => return Err(CommandError::bad("id", "cannot be deleted here")),
                }
                Ok(Value::Null)
            }
            "doc.picture" => {
                self.rebuild();
                let document = &self.document;
                crate::app::scripts::picture_command(&args, |request| {
                    crate::proof::picture(document, request, None, axes::AxisSystem::default())
                })
            }
            "file.import" => {
                let path = PathBuf::from(a.string("path")?);
                self.import(&path)?;
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
                let format = crate::app::scripts::export_format(a.opt_string("format")?, &path)?;
                let bodies = crate::app::scripts::body_list(args.get("bodies"))?;
                self.rebuild();
                let layout = (a.opt_bool("layout")? == Some(true)).then(|| {
                    use crate::app::print_layout::{LayoutSettings, lay_out, parts_to_print};
                    lay_out(
                        &self.document,
                        &parts_to_print(&self.document, &self.registry),
                        &LayoutSettings::of(&settings::PrintingSettings::default()),
                    )
                });
                let (path, exported) = crate::app::export::export_document(
                    &self.document,
                    path,
                    format,
                    bodies,
                    a.opt_number("tolerance")?.map(|t| t as f32),
                    layout.as_ref(),
                )
                .map_err(CommandError::failed)?;
                Ok(json!({
                    "path": path.display().to_string(),
                    "written": exported.written,
                    "skipped": exported.skipped,
                    "triangles": exported.triangles,
                }))
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

impl Headless {
    /// Build every solid that changed, here and now, until nothing is left
    /// to build. The failures, as `{feature, name, error}`.
    fn rebuild(&mut self) -> Vec<Value> {
        let mut kernel = kernel_ogeom::OgeomKernel::new();
        // A plan that fails keeps its body's features clean, so the loop
        // ends; the bound is a guard, not the expected way out.
        let mut read = std::collections::HashSet::new();
        for _ in 0..32 {
            self.read_shapes(&mut read);
            // What stands on a solid just built follows it before the next
            // round is planned.
            self.registry.evaluate(&mut self.document);
            let jobs = self.registry.rebuild_jobs(&mut self.document);
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                let body = job.body;
                self.document.clear_body_feature_errors(body);
                let mut plan = match job.plan {
                    Ok(plan) => plan,
                    Err(err) => {
                        if let Some(feature) = err.feature {
                            self.document
                                .set_feature_error(feature, Some(err.message.clone()));
                        }
                        continue;
                    }
                };
                if let Some(err) = plan.failed.take()
                    && let Some(feature) = err.feature
                {
                    self.document.set_feature_error(feature, Some(err.message));
                    let unbuilt = std::mem::take(&mut plan.unbuilt);
                    crate::app::recompute::mark_unbuilt(&mut self.document, feature, unbuilt);
                }
                // A step that fails leaves the body the history before it.
                loop {
                    if plan.ops.is_empty() {
                        if !self.document.body_solid_is_imported(body) {
                            self.document.remove_imported_geometry(body);
                        }
                        break;
                    }
                    let asked: Vec<kernel_api::ChainProbe> =
                        plan.probes.iter().map(|p| p.probe).collect();
                    let tags: Vec<kernel_api::TopoName> = plan
                        .op_features
                        .iter()
                        .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
                        .collect();
                    let projections: Vec<kernel_api::ChainProjection> =
                        plan.projections.iter().map(|p| p.projection).collect();
                    match kernel.execute_solid_chain_projecting(
                        &plan.ops,
                        &tags,
                        &TessellationSettings::default(),
                        None,
                        &asked,
                        &projections,
                    ) {
                        Ok(result) => {
                            self.document
                                .store_probe_answers(&plan.probes, &result.probes);
                            self.document
                                .store_projections(&plan.projections, &result.projections);
                            crate::app::recompute::store_built_solid(
                                &mut self.document,
                                body,
                                result,
                            );
                            break;
                        }
                        Err(err) => {
                            let Some(feature) = plan.op_features.get(err.op_index).copied() else {
                                break;
                            };
                            self.document
                                .set_feature_error(feature, Some(err.message.clone()));
                            let start = plan
                                .op_features
                                .iter()
                                .position(|f| *f == feature)
                                .unwrap_or(0);
                            let after = plan.op_features[start..].to_vec();
                            crate::app::recompute::mark_unbuilt(&mut self.document, feature, after);
                            plan.ops.truncate(start);
                            plan.op_features.truncate(start);
                            plan.probes.retain(|p| p.probe.after_op <= start);
                            plan.projections.retain(|p| p.projection.after_op <= start);
                        }
                    }
                }
            }
        }
        self.read_shapes(&mut read);
        self.mirror_copies();
        crate::app::scripts::rebuild_failures(&self.document)
    }

    /// Read every shape the document takes from a file and has not yet
    /// (`Document::bodies_awaiting_shape`): shapes replaced or given from
    /// a file, and the stand-ins of bodies that do not build. Each is read
    /// once a rebuild.
    fn read_shapes(
        &mut self,
        read: &mut std::collections::HashSet<(core_document::BodyId, uuid::Uuid)>,
    ) {
        let mut kernel = kernel_ogeom::OgeomKernel::new();
        let detail = TessellationSettings::default();
        for (body, asset) in self.document.bodies_awaiting_shape() {
            if !read.insert((body, asset)) {
                continue;
            }
            let Some((reference, bytes)) = self.document.asset_with_bytes(asset) else {
                continue;
            };
            let name = Path::new(&reference.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let shape = crate::platform::scratch_file("shapes", &name, &bytes)
                .map_err(|e| e.to_string())
                .and_then(|path| {
                    let shape = kernel.read_solid(&path, &detail).map_err(|e| e.to_string());
                    let _ = std::fs::remove_file(path);
                    shape
                });
            match shape {
                Ok(shape) => {
                    crate::app::recompute::land_shape(
                        &mut self.document,
                        &self.registry,
                        body,
                        asset,
                        shape,
                    );
                }
                Err(error) => eprintln!("a shape could not be read from {name}: {error}"),
            }
        }
    }

    /// Give every mirrored copy without one its snapshot: its source's,
    /// mirrored by the kernel.
    fn mirror_copies(&mut self) {
        use kernel_api::KernelQueries;
        for (body, from, plane) in self.document.copies_awaiting_shape() {
            match kernel_ogeom::QUERIES.mirror(
                &from,
                plane.point.map(f64::from),
                plane.normal.map(f64::from),
            ) {
                Ok(blob) => {
                    self.document.set_mirrored_shape(body, &from, blob);
                }
                Err(error) => {
                    let name = self
                        .document
                        .bodies()
                        .iter()
                        .find(|b| b.id == body)
                        .map_or("a mirrored copy", |b| b.name.as_str());
                    eprintln!("`{name}` could not be mirrored: {error}");
                }
            }
        }
    }

    /// Delete a feature the way its workbench deletes it.
    fn delete_feature(&mut self, feature: FeatureId) -> Result<(), CommandError> {
        let kind = self
            .document
            .get_feature_meta(feature)
            .map(|n| n.workbench_id.clone())
            .ok_or_else(|| CommandError::bad("id", "is not in this document"))?;
        let body = self.document.get_feature_meta(feature).and_then(|n| n.body);
        let deleted = match self.registry.owner_id_of(&kind).cloned() {
            Some(owner) => {
                let wb = self
                    .registry
                    .workbench_mut(&owner)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let mut ctx = context(&mut self.document);
                wb.delete_feature(&mut ctx, feature)
            }
            None => self.document.remove_feature(feature).is_ok(),
        };
        if deleted {
            // The body rebuilds from what is left, or drops the shape
            // nothing builds any more.
            if let Some(body) = body {
                self.registry.invalidate_body(&mut self.document, body);
            }
            Ok(())
        } else {
            Err(CommandError::failed("the feature could not be deleted"))
        }
    }

    /// Read `path` into the document as the app's import does: a file
    /// kind a workbench reads goes to its command; anything else the
    /// kernel reads here, on this thread, and lands as the app lands it.
    fn import(&mut self, path: &Path) -> Result<(), CommandError> {
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
        use kernel_api::Kernel;
        let detail = kernel_api::TessellationSettings::default();
        let model = kernel_ogeom::OgeomKernel::new()
            .import_step(path, &detail)
            .map_err(|e| CommandError::failed(e.to_string()))?;
        let bytes = std::fs::read(path).map_err(|e| CommandError::failed(e.to_string()))?;
        let landed =
            crate::app::step_import::land_import(&mut self.document, path, model, bytes, detail);
        if landed.bodies.is_empty() {
            return Err(CommandError::failed("the file held no geometry"));
        }
        Ok(())
    }

    fn save(&mut self, path: &Path) -> Result<()> {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.document
                .set_name(crate::app::doc_io::document_name_from_file_name(name));
        }
        self.document
            .save_to_file(path, core_document::Compression::None)?;
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
    /// a printCAD file, a linked part, waits for the app.
    #[test]
    fn a_script_imports_a_step_file() {
        let step = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kernel_ogeom/tests/data/box_native.step")
            .display()
            .to_string();
        let source = format!(
            r#"
            pc.file.import{{path = [[{step}]]}}
            local bodies = pc.doc.bodies()
            assert(#bodies == 1, #bodies)
            assert(math.abs(pc.doc.measure{{body = bodies[1].id}}.volume - 24000) < 1e-6)
            assert(not pcall(pc.file.import, {{path = "part.prtcad"}}))
            "#
        );
        let mut registry = registry();
        crate::headless::run_in_empty_document(&mut registry, &source, "import").unwrap();
    }

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
        let export = crate::app::export::every_configuration_script(
            &dir.join("part").display().to_string(),
            "stl",
            0.05,
            "nil",
            false,
        );
        let check = r#"
            assert(pc.config.list().active == "Small", "the active one is back")
        "#;
        std::fs::write(&script, format!("{setup}\n{export}\n{check}")).unwrap();
        assert!(run_file(script, None, Vec::new()), "the script runs");
        assert!(dir.join("part-Small.stl").exists());
        assert!(dir.join("part-Large two.stl").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
