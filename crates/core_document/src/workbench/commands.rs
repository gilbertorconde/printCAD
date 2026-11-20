//! The commands a bench runs for scripts, agents and File › Import.

use crate::runtime::WorkbenchRuntimeContext;

/// Running the commands a bench registered with
/// [`WorkbenchContext::register_command`](super::WorkbenchContext::register_command),
/// which scripts, agents and the file imports naming one call.
pub trait BenchCommands {
    /// Run one of the commands this bench registered with
    /// `WorkbenchContext::register_command`. The host has checked `args`
    /// against the command's spec. A command that makes something answers
    /// its id; it never opens a task or waits for a click.
    fn run_command(
        &mut self,
        id: &str,
        _args: &crate::CommandArgs,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> crate::CommandResult {
        Err(crate::CommandError::Unknown(id.to_string()))
    }
}
