//! What the Surface commands take and what a caller should know of them:
//! one command per kind of step, with its fields as arguments, plus
//! `surface.check` and `surface.set`, each with notes, related commands
//! and a working example.

mod checks;
mod set;
mod steps;

use core_document::{CommandArgs, CommandResult, WorkbenchContext, WorkbenchRuntimeContext};

use crate::SurfaceWorkbench;

/// Register every Surface command, in the order the reference lists them.
pub(crate) fn configure(context: &mut WorkbenchContext) {
    steps::configure(context);
    checks::configure(context);
    set::configure(context);
}

/// Run command `id` on `bench`.
pub(crate) fn run(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if checks::owns(id) {
        return checks::run(bench, id, args, ctx);
    }
    if set::owns(id) {
        return set::run(args, ctx);
    }
    steps::run(bench, id, args, ctx)
}
