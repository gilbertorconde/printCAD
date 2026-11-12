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
    CommandArgs, CommandResult, CommandTable, WorkbenchContext, WorkbenchRuntimeContext,
};
use std::sync::OnceLock;

use crate::{InspectionWorkbench, align_commands};

/// Every command, in the order the reference lists them.
fn table() -> &'static CommandTable<InspectionWorkbench> {
    static TABLE: OnceLock<CommandTable<InspectionWorkbench>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = CommandTable::default();
        t.include(align_commands::family());
        sections::table(&mut t);
        maps::table(&mut t);
        output::table(&mut t);
        tolerances::table(&mut t);
        walls::table(&mut t);
        comparisons::table(&mut t);
        alignment::table(&mut t);
        elements::table(&mut t);
        t
    })
}

/// Register every command, in the order the reference lists them.
pub(crate) fn configure(context: &mut WorkbenchContext) {
    table().register(context);
}

/// Run command `id` on `bench`.
pub(crate) fn run(
    bench: &mut InspectionWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    table().run_on(bench, id, args, ctx)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_section_of_the_inspection_prose_names_a_command() {
        let bench = &mut crate::InspectionWorkbench::default();
        for prose in [
            super::alignment::prose(),
            super::comparisons::prose(),
            super::elements::prose(),
            super::maps::prose(),
            super::output::prose(),
            super::sections::prose(),
            super::tolerances::prose(),
            super::walls::prose(),
            crate::align_commands::prose(),
        ] {
            core_document::testing::assert_prose_names_commands(bench, prose);
        }
    }
}
