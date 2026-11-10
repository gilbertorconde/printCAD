//! Inspection's commands, one family to a file, each with its specs and
//! what runs them.

mod alignment;
mod comparisons;
mod elements;
mod maps;
mod output;
mod sections;
mod tolerances;
mod walls;

use core_document::{
    CommandArgs, CommandError, CommandResult, WorkbenchContext, WorkbenchRuntimeContext,
};

use crate::InspectionWorkbench;

/// Register every family's commands, in the order the reference lists them.
pub(crate) fn configure(context: &mut WorkbenchContext) {
    sections::configure(context);
    maps::configure(context);
    output::configure(context);
    tolerances::configure(context);
    walls::configure(context);
    comparisons::configure(context);
    alignment::configure(context);
    elements::configure(context);
}

/// Run command `id` on `bench`.
pub(crate) fn run(
    bench: &InspectionWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if sections::owns(id) {
        return sections::run(id, args, ctx);
    }
    if maps::owns(id) {
        return maps::run(bench, id, args, ctx);
    }
    if output::owns(id) {
        return output::run(bench, id, args, ctx);
    }
    if tolerances::owns(id) {
        return tolerances::run(id, args, ctx);
    }
    if walls::owns(id) {
        return walls::run(bench, id, args, ctx);
    }
    if comparisons::owns(id) {
        return comparisons::run(bench, id, args, ctx);
    }
    if alignment::owns(id) {
        return alignment::run(id, args, ctx);
    }
    if elements::owns(id) {
        return elements::run(bench, id, args, ctx);
    }
    Err(CommandError::Unknown(id.to_string()))
}
