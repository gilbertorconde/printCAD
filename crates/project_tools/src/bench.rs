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

use crate::document_import::{BodyKind, ImportedObject, convert_into};

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
                "Bring a FreeCAD file's bodies, sketches, texts and links into the document \
                 as bodies of Design features, sketches and linked copies, and what has no \
                 counterpart as the shapes the file saved",
            )
            .param("path", ParamKind::String, "The .FCStd file")
            .returns("{bodies = [{id, name, kind, features}], sketches, findings}")
            .agent_always_asks()
            .note(
                "Each body of the file becomes one body: its features in their order, its \
                 tip the file's. Operations outside bodies continue the body of the solid \
                 they start from when nothing else uses it, else start one with a Clone \
                 of it. Sketches keep their geometry and the constraints printCAD has, \
                 placed where the file places them; a text becomes a sketch's text block. \
                 The edges a fillet or chamfer rounds and the faces an extrusion ends on \
                 are found on the shapes the file saved. A link becomes a linked copy.",
            )
            .note(
                "What printCAD cannot rebuild as features (a solid kept without its \
                 history, a kind or setting it has no counterpart for) comes in as the \
                 shape the file saved for it, `kind` \"saved_shape\"; each body built \
                 from features keeps its tip's saved shape to show while its history \
                 does not build. Attachments and formulas are frozen to the values the \
                 file saved. Each of these is named in the log and in `findings`. An \
                 import that keeps shapes from the file cannot be undone, as a STEP \
                 import cannot.",
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
    let bytes = std::fs::read(path)
        .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
    let archive = FcstdArchive::from_bytes(&bytes)
        .map_err(|e| CommandError::bad("path", format!("is not a project archive: {e}")))?;
    let source = FcDocument::parse(&archive.document_xml)
        .map_err(|e| CommandError::bad("path", e.to_string()))?;
    // A conversion stopped by the file's own data part way would leave what
    // came before it, so it runs on a scratch document first.
    let trial = convert_into(&source, &archive.shapes, &mut Document::new("scratch"))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if trial.objects.is_empty() {
        for finding in &trial.findings {
            ctx.log_warn(format!("{}: {}", finding.object, finding.reason));
        }
        return Err(CommandError::failed(
            "the file holds nothing printCAD can bring in",
        ));
    }
    let conversion = convert_into(&source, &archive.shapes, ctx.document)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let sketches: Vec<String> = conversion
        .objects
        .values()
        .filter_map(|o| match o {
            ImportedObject::Sketch(id) => Some(id.0.to_string()),
            ImportedObject::Solid { .. } => None,
        })
        .collect();
    let count = |kind| conversion.bodies.iter().filter(|b| b.kind == kind).count();
    ctx.log_info(format!(
        "Imported {}: {} bodies ({} built from features, {} kept as saved shapes, {} linked \
         copies) and {} sketches",
        path.display(),
        conversion.bodies.len(),
        count(BodyKind::Features),
        count(BodyKind::SavedShape),
        count(BodyKind::LinkedCopy),
        sketches.len()
    ));
    let bodies: Vec<serde_json::Value> = conversion
        .bodies
        .iter()
        .map(|b| {
            json!({ "id": b.body.0.to_string(), "name": b.name, "kind": b.kind, "features": b.features })
        })
        .collect();
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
