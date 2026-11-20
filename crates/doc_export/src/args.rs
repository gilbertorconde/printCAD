//! A script's arguments for an export, read into the dialog's options and
//! written back from them.

use core_document::{Args, CommandArgs, CommandError};
use serde_json::json;

/// The format an export names, else the one its path's extension says.
pub fn export_format(
    name: Option<&str>,
    path: &std::path::Path,
) -> Result<kernel_ogeom::export::ExportFormat, CommandError> {
    use kernel_ogeom::export::ExportFormat;
    match name {
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "step" | "stp" => Ok(ExportFormat::Step),
            "step_nurbs" => Ok(ExportFormat::StepNurbs),
            "stl" => Ok(ExportFormat::Stl),
            "3mf" => Ok(ExportFormat::ThreeMf),
            _ => Err(CommandError::bad(
                "format",
                "must be step, step_nurbs, stl or 3mf",
            )),
        },
        None => ExportFormat::of_path(path).ok_or_else(|| {
            CommandError::bad(
                "format",
                "is needed when the path has no .step, .stl or .3mf",
            )
        }),
    }
}

/// The name `export_format` reads for `format`.
pub fn format_name(format: kernel_ogeom::export::ExportFormat) -> &'static str {
    use kernel_ogeom::export::ExportFormat;
    match format {
        ExportFormat::Step => "step",
        ExportFormat::StepNurbs => "step_nurbs",
        ExportFormat::Stl => "stl",
        ExportFormat::ThreeMf => "3mf",
    }
}

/// The export a script's arguments ask for, as the File › Export dialog's
/// options: each one left out as the dialog first opens on it.
pub fn export_draft(a: &Args, path: &std::path::Path) -> Result<crate::ExportDraft, CommandError> {
    let mut draft = crate::ExportDraft {
        format: export_format(a.opt_string("format")?, path)?,
        ..Default::default()
    };
    if let Some(tolerance) = a.opt_number("tolerance")? {
        draft.detail.chord_tolerance = tolerance as f32;
    }
    if let Some(angle) = a.opt_number("angle")? {
        draft.detail.angular_tolerance_deg = angle as f32;
    }
    if let Some(textures) = a.opt_bool("textures")? {
        draft.textures = textures;
    }
    draft.layout = a.opt_bool("layout")? == Some(true);
    draft.per_body = a.opt_bool("per_body")? == Some(true);
    Ok(draft)
}

/// The arguments that ask `export_draft` for what `draft` writes.
pub fn export_args(draft: &crate::ExportDraft) -> CommandArgs {
    let mut args = CommandArgs::new();
    args.insert("format".into(), json!(format_name(draft.format)));
    args.insert("tolerance".into(), json!(draft.detail.chord_tolerance));
    args.insert("angle".into(), json!(draft.detail.angular_tolerance_deg));
    args.insert("textures".into(), json!(draft.textures));
    args.insert("layout".into(), json!(draft.layout));
    if draft.per_body {
        args.insert("per_body".into(), json!(true));
    }
    args
}
