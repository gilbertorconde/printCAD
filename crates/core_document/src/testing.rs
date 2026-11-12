//! What benches' tests share to run commands and hooks without a window:
//! a context with no camera to speak of, and a command called as a
//! script calls it. Built only with the `test-support` feature, which
//! only dev-dependencies turn on.

use serde_json::Value;

use crate::command::{CommandResult, object};
use crate::{Document, Workbench, WorkbenchContext, WorkbenchRuntimeContext};

/// A context on `doc` with the camera at the origin and a one-pixel
/// viewport: for calls that read neither.
pub fn ctx(doc: &mut Document) -> WorkbenchRuntimeContext<'_> {
    WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1))
}

/// Run command `id` of `bench` on `doc` as a script does: `args`, a JSON
/// object, checked against the spec the bench registers first.
///
/// Panics when the bench registers no such command or `args` is not an
/// object.
pub fn call_checked(
    bench: &mut dyn Workbench,
    doc: &mut Document,
    id: &str,
    args: Value,
) -> CommandResult {
    assert!(args.is_object(), "`{id}`'s arguments are an object");
    let args = object(args);
    let mut registered = WorkbenchContext::default();
    bench.configure(&mut registered);
    let spec = registered
        .commands()
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("`{id}` is registered"));
    spec.check(&args)?;
    bench.run_command(id, &args, &mut ctx(doc))
}
