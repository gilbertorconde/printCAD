//! File › Import for project archives: a workbench with no tools and no
//! feature kinds of its own, registering the `.FCStd` import and the
//! command it runs. What it brings in is ordinary sketches, Design
//! features and bodies, which the benches owning them build and edit.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FileImport, ParamKind,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchRuntimeContext,
};
use project_archive::archive::FcstdArchive;
use project_archive::document_xml::FcDocument;
use serde_json::json;

use crate::document_import::{ImportedObject, convert_into};

/// The command File › Import runs for a project archive.
pub const IMPORT_COMMAND: &str = "project.import";

/// Registers the project archive import; it shows in no bench switcher.
#[derive(Debug, Default)]
pub struct ProjectImportBench;

impl Workbench for ProjectImportBench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.project",
            "Project import",
            "Brings sketches, features and bodies in from project archives",
        )
        .icon("file-document")
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        context.register_command(
            CommandSpec::new(
                IMPORT_COMMAND,
                "Bring a FreeCAD file's sketches, Pad, Pocket, primitive and boolean \
                 features into the document as sketches, Design features and bodies",
            )
            .param("path", ParamKind::String, "The .FCStd file")
            .returns("{bodies, sketches, findings}")
            .agent_always_asks()
            .note(
                "Each solid feature of the file becomes a body of its own whose history \
                 clones the body before it, so every step stays addressable; the earlier \
                 steps are hidden. Sketches keep their geometry and the constraints \
                 printCAD has, placed where the file places them.",
            )
            .note(
                "A feature printCAD cannot rebuild (a fillet, a tapered or two-sided pad) is \
                 left out with everything built on it, and attachments and formulas are \
                 frozen to the values the file saved; each is named in the log and in \
                 `findings`. The rest comes in, as one undo step.",
            )
            .note(
                "A file that cannot be read, or holds nothing that can be brought in, is \
                 refused and the document is left as it was.",
            ),
        );
        context.register_import(FileImport::new("FreeCAD file", ["fcstd"], IMPORT_COMMAND));
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        match id {
            IMPORT_COMMAND => import(&Args(args), ctx),
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

fn import(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let path = std::path::Path::new(a.string("path")?);
    let bytes = kernel_ogeom::files::read(path)
        .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
    let archive = FcstdArchive::from_bytes(&bytes)
        .map_err(|e| CommandError::bad("path", format!("is not a project archive: {e}")))?;
    let source = FcDocument::parse(&archive.document_xml)
        .map_err(|e| CommandError::bad("path", e.to_string()))?;
    // A conversion stopped by the file's own data part way would leave what
    // came before it, so it runs on a scratch document first.
    let trial = convert_into(&source, &mut Document::new("scratch"))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if trial.objects.is_empty() {
        for finding in &trial.findings {
            ctx.log_warn(format!("{}: {}", finding.object, finding.reason));
        }
        return Err(CommandError::failed(
            "the file holds nothing printCAD can bring in",
        ));
    }
    let conversion =
        convert_into(&source, ctx.document).map_err(|e| CommandError::failed(e.to_string()))?;
    let sketches: Vec<String> = conversion
        .objects
        .values()
        .filter_map(|o| match o {
            ImportedObject::Sketch(id) => Some(id.0.to_string()),
            ImportedObject::Solid(_) => None,
        })
        .collect();
    let bodies: Vec<String> = conversion
        .solids
        .iter()
        .map(|(_, body)| body.0.to_string())
        .collect();
    ctx.log_info(format!(
        "Imported {}: {} sketch(es) and {} solid(s)",
        path.display(),
        sketches.len(),
        bodies.len()
    ));
    for finding in &conversion.findings {
        ctx.log_warn(format!(
            "{}: {} ({}): {}",
            path.display(),
            finding.object,
            finding.category,
            finding.reason
        ));
    }
    Ok(json!({
        "bodies": bodies,
        "sketches": sketches,
        "findings": conversion.findings,
    }))
}
