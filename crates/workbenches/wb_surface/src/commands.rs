//! What the Surface commands take and what a caller should know of them:
//! one command per kind of step, with its fields as arguments, plus
//! `surface.check` and `surface.set`, each with notes, related commands
//! and a working example.

mod checks;
mod set;
mod steps;

use core_document::{
    CommandArgs, CommandResult, CommandTable, WorkbenchContext, WorkbenchRuntimeContext,
};
use std::sync::OnceLock;

use crate::SurfaceWorkbench;

/// Every Surface command, in the order the reference lists them.
fn table() -> &'static CommandTable<SurfaceWorkbench> {
    static TABLE: OnceLock<CommandTable<SurfaceWorkbench>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = CommandTable::default();
        steps::table(&mut t);
        checks::table(&mut t);
        set::table(&mut t);
        t
    })
}

/// Register every Surface command, in the order the reference lists them.
pub(crate) fn configure(context: &mut WorkbenchContext) {
    table().register(context);
}

/// Run command `id` on `bench`.
pub(crate) fn run(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    table().run_on(bench, id, args, ctx)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_section_of_the_surface_prose_names_a_command() {
        let bench = &mut crate::SurfaceWorkbench::default();
        for prose in [
            super::steps::prose(),
            super::checks::prose(),
            super::set::prose(),
        ] {
            core_document::testing::assert_prose_names_commands(bench, prose);
        }
    }
}
