//! Running a script without a window: `printcad --script build.lua`, for
//! batch exports, checks and tests.
//!
//! The script sees the workbenches' commands and the document's own
//! (reading it, adding and naming bodies, visibility, deleting, rebuilding
//! and measuring), and `file.save_as` and `file.export` with a path. What
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
    engine.set_args(&invocation.args);
    let name = invocation.script.display().to_string();
    let out = engine.run_script(&source, &name, &mut host);
    for line in &out.printed {
        println!("{line}");
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

/// The document and the workbenches, with no window.
struct Headless {
    registry: DocumentService,
    document: Document,
    file: Option<PathBuf>,
}

/// The document commands a window-less run answers.
const DOC_COMMANDS: &[&str] = &[
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
    "doc.measure",
];

fn file_commands() -> Vec<CommandSpec> {
    vec![
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
        let spec = scripting::Host::commands(self)
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| match doc_commands().iter().any(|c| c.id == id) {
                true => CommandError::failed("not available without a window"),
                false => CommandError::Unknown(id.to_string()),
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
                    _ => return Err(CommandError::bad("id", "cannot be deleted here")),
                }
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
            "file.export" => {
                let path = PathBuf::from(a.string("path")?);
                let format = crate::app::scripts::export_format(a.opt_string("format")?, &path)?;
                let bodies = crate::app::scripts::body_list(args.get("bodies"))?;
                self.rebuild();
                let (path, exported) = crate::app::export::export_document(
                    &self.document,
                    path,
                    format,
                    bodies,
                    a.opt_number("tolerance")?.map(|t| t as f32),
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
                let wb = self
                    .registry
                    .workbench_mut(&bench)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let mut ctx = context(&mut self.document);
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
        for _ in 0..32 {
            let jobs = self.registry.rebuild_jobs(&mut self.document);
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                let body = job.body;
                self.document.clear_body_feature_errors(body);
                match job.plan {
                    Ok(plan) if plan.ops.is_empty() => {
                        if !self.document.body_solid_is_imported(body) {
                            self.document.remove_imported_geometry(body);
                        }
                    }
                    Ok(plan) => {
                        match kernel
                            .execute_solid_chain(&plan.ops, &TessellationSettings::default())
                        {
                            Ok(result) => crate::app::recompute::store_built_solid(
                                &mut self.document,
                                body,
                                result,
                            ),
                            Err(err) => {
                                if let Some(feature) = plan.op_features.get(err.op_index) {
                                    self.document
                                        .set_feature_error(*feature, Some(err.message.clone()));
                                }
                            }
                        }
                    }
                    Err(err) => {
                        if let Some(feature) = err.feature {
                            self.document
                                .set_feature_error(feature, Some(err.message.clone()));
                        }
                    }
                }
            }
        }
        let mut nodes: Vec<_> = self
            .document
            .feature_tree()
            .all_nodes()
            .map(|(_, n)| n)
            .collect();
        nodes.sort_by_key(|n| n.seq);
        nodes
            .into_iter()
            .filter_map(|n| {
                n.error
                    .as_ref()
                    .map(|e| json!({"feature": n.id.0.to_string(), "name": n.name, "error": e}))
            })
            .collect()
    }

    /// Delete a feature the way its workbench deletes it.
    fn delete_feature(&mut self, feature: FeatureId) -> Result<(), CommandError> {
        let kind = self
            .document
            .get_feature_meta(feature)
            .map(|n| n.workbench_id.clone())
            .ok_or_else(|| CommandError::bad("id", "is not in this document"))?;
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
            Ok(())
        } else {
            Err(CommandError::failed("the feature could not be deleted"))
        }
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

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
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

    #[test]
    fn a_script_builds_measures_exports_and_saves_without_a_window() {
        let mut registry = DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let dir = std::env::temp_dir().join(format!("printcad-headless-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("block.lua");
        std::fs::write(
            &script,
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.part.pad{sketch = s, length = tonumber(arg[1])}
            assert(#pc.doc.rebuild() == 0, "the pad builds")
            local body = pc.doc.bodies()[1].id
            local m = pc.doc.measure{body = body}
            print(string.format("%.1f", m.volume))
            pc.file.export{path = arg[2]}
            "#,
        )
        .unwrap();
        let stl = dir.join("block.stl");
        let saved = dir.join("block.prtcad");
        let ok = run(
            &Invocation {
                script,
                open: None,
                save: Some(saved.clone()),
                args: vec!["5".into(), stl.display().to_string()],
            },
            registry,
        )
        .unwrap();
        assert!(ok);
        assert!(
            std::fs::metadata(&stl).unwrap().len() > 84,
            "an STL with triangles"
        );
        let reopened = Document::load_from_file(&saved).unwrap();
        assert_eq!(reopened.bodies().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
