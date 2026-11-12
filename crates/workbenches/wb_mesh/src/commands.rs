//! The Mesh bench's commands: one table, each spec beside what runs it.
//! Most work on a mesh body and its mesh as it started, which
//! `on_scan` finds for them.
use super::*;
use std::sync::OnceLock;

/// What a command does on the mesh body it works on: told its id, the
/// body and the body's mesh as it started.
pub(super) type ScanStep =
    fn(&str, &Args, &mut WorkbenchRuntimeContext, BodyId, &TriMesh) -> CommandResult;

/// Add `spec`, run by `step` on the mesh body its `scan` argument names
/// (the selected body when left out) and that body's starting mesh,
/// refused when the body has none.
pub(super) fn on_scan(t: &mut CommandTable<MeshWorkbench>, spec: CommandSpec, step: ScanStep) {
    let id = spec.id.clone();
    t.add_free(spec, move |args, ctx| {
        let a = Args(args);
        let body = target(&a, ctx)?;
        let start = start_mesh(ctx.document, body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        step(&id, &a, ctx, body, &start)
    });
}

/// Every command, in the order the reference lists them.
fn table() -> &'static CommandTable<MeshWorkbench> {
    static TABLE: OnceLock<CommandTable<MeshWorkbench>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = CommandTable::default();
        steps::table(&mut t);
        clean_up(&mut t);
        t
    })
}

/// Register every command.
pub(super) fn register(context: &mut WorkbenchContext) {
    table().register(context);
}

/// Run command `id` on `bench`.
pub(super) fn run(
    bench: &mut MeshWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    table().run_on(bench, id, args, ctx)
}

/// The step a tool of `steps::TOOLS` makes, worked out from the mesh the
/// body draws now.
pub(super) fn made(
    id: &str,
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let (_, label, _) = steps::TOOLS
        .iter()
        .find(|(tool, _, _)| *tool == id)
        .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
    let (mesh, _) = ctx
        .document
        .local_geometry(body)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let op = steps::make(id, a, ctx, body, &mesh)?;
    add_step(ctx, body, label, op)
}

/// Removing specks, deleting, flipping and cutting, and reading the edit.
fn clean_up(t: &mut CommandTable<MeshWorkbench>) {
    let scan =
        |spec: CommandSpec| spec.optional("scan", ParamKind::Id, "The mesh body; the selected one");
    on_scan(
        t,
        scan(CommandSpec::new(ISLANDS, "Remove specks"))
            .optional("smaller_than", ParamKind::Number, "Pieces whose box's diagonal is shorter than this go, mm; a hundredth of the mesh's")
            .returns("{feature, name}"),
        islands,
    );
    on_scan(
        t,
        scan(CommandSpec::new(DELETE, "Delete marked faces"))
            .optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
            .optional(
                "box",
                ParamKind::Any,
                "{min = {x, y, z}, max = {x, y, z}} in the body's frame",
            )
            .optional(
                "ball",
                ParamKind::Any,
                "{centre = {x, y, z}, radius = r} in the body's frame",
            )
            .returns("{feature, name}"),
        delete,
    );
    on_scan(
        t,
        scan(CommandSpec::new(FLIP, "Flip normals"))
            .optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
            .optional("box", ParamKind::Any, "Only these faces")
            .optional("ball", ParamKind::Any, "Only these faces")
            .returns("{feature, name}"),
        flip,
    );
    on_scan(
        t,
        scan(CommandSpec::new(CUT, "Cut by a plane"))
            .optional("plane", ParamKind::String, "xy, xz or yz, in the body's frame; xy")
            .optional("at", ParamKind::Number, "Where along the plane's normal, mm; the mesh's middle")
            .optional("keep", ParamKind::String, "below or above the plane; below")
            .optional("close", ParamKind::Bool, "Cap every loop of the cut flat; true")
            .optional("bridge", ParamKind::Number, "Gaps in the cut's rim up to this are closed straight, mm; a fiftieth of the mesh's diagonal")
            .returns("{feature, name}"),
        cut,
    );
    on_scan(
        t,
        scan(CommandSpec::new(DEVIATION, "Edit deviation"))
            .returns("{max, rms, p99, added}")
            .read_only(),
        deviation_of_edit,
    );
    on_scan(
        t,
        scan(CommandSpec::new(INSPECT, "Topology and volume"))
            .returns("{triangles, open_edges, volume, pieces, bounds = {min, max}}")
            .read_only(),
        topology,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "commands.md", include_str!("commands.md"), &[])
}

fn islands(
    _: &str,
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    start: &TriMesh,
) -> CommandResult {
    let smaller_than = a
        .opt_number("smaller_than")?
        .unwrap_or(diagonal(start) / 100.0);
    add_step(
        ctx,
        body,
        "Remove islands",
        MeshOp::RemoveIslands { smaller_than },
    )
}

fn delete(
    _: &str,
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let region = region(a, ctx, body, false)?;
    add_step(ctx, body, "Delete", MeshOp::Delete { region })
}

fn flip(
    _: &str,
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let region = region(a, ctx, body, true)?;
    add_step(ctx, body, "Flip", MeshOp::Flip { region })
}

fn cut(
    _: &str,
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let (now, _) = ctx
        .document
        .local_geometry(body)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let op = solids::cut(a, &now)?;
    add_step(ctx, body, "Cut", op)
}

fn deviation_of_edit(
    _: &str,
    _a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let start = ctx
        .document
        .mesh_start(body)
        .ok_or_else(|| CommandError::failed("the scan has no edits"))?
        .mesh
        .clone();
    let (now, _) = ctx
        .document
        .local_geometry(body)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
    Ok(deviation_reading(
        &deviation(kernel, &start, &now).map_err(CommandError::failed)?,
    ))
}

fn topology(
    _: &str,
    _a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    _start: &TriMesh,
) -> CommandResult {
    let (drawn, _) = ctx
        .document
        .local_geometry(body)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    Ok(inspect(&drawn))
}
