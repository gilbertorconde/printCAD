//! The Mesh bench: a mesh body's edits as a history of features on it,
//! each one step (`kernel_api::MeshOp`) the host builds from the body's
//! mesh start on a thread, in order, the result the body's mesh. Every
//! edit is undoable, editable and saved; the history's start is the mesh
//! as imported.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentError,
    DocumentResult, FeatureId, FeatureInfo, FeatureNode, InputResult, MeshPlan, ParamKind,
    ToolDescriptor, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId,
    WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use kernel_api::{MeshOp, MeshRegion, TriMesh};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const ID: &str = "wb.mesh";
/// The feature kind of every mesh step.
pub const KIND: &str = "wb.mesh";

const ISLANDS: &str = "mesh.islands";
const DELETE: &str = "mesh.delete";
const FLIP: &str = "mesh.flip";
const CUT: &str = "mesh.cut";
const INSPECT: &str = "mesh.inspect";

/// One step of a mesh body's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshFeature {
    pub name: String,
    pub scan: BodyId,
    pub op: MeshOp,
    /// A cut's plane moved this far along its normal, mm: what the
    /// property panel edits.
    #[serde(default)]
    pub offset: f64,
}

impl WorkbenchFeature for MeshFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl MeshFeature {
    /// Its step as the kernel takes it, the offset applied.
    pub fn step(&self) -> MeshOp {
        match &self.op {
            MeshOp::Cut {
                origin,
                normal,
                keep_above,
                close,
                bridge,
            } => {
                let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                    .sqrt()
                    .max(1e-12);
                MeshOp::Cut {
                    origin: std::array::from_fn(|k| origin[k] + normal[k] / len * self.offset),
                    normal: *normal,
                    keep_above: *keep_above,
                    close: *close,
                    bridge: *bridge,
                }
            }
            other => other.clone(),
        }
    }
}

#[derive(Default)]
pub struct MeshWorkbench;

/// The scan a command or tool works on: `scan` given, else the selected
/// mesh body.
fn target(a: &Args, ctx: &WorkbenchRuntimeContext) -> Result<BodyId, CommandError> {
    let body = match a.opt_id("scan")? {
        Some(id) => BodyId(id),
        None => ctx
            .selected_body_id
            .map(BodyId)
            .ok_or_else(|| CommandError::bad("scan", "names no mesh body, and none is selected"))?,
    };
    if !ctx.document.is_mesh_body(body) && ctx.document.mesh_start(body).is_none() {
        return Err(CommandError::bad("scan", "is not a mesh body"));
    }
    Ok(body)
}

/// The mesh a history starts from: the start kept, else the body's mesh.
fn start_mesh(document: &Document, body: BodyId) -> Option<std::sync::Arc<TriMesh>> {
    document
        .mesh_start(body)
        .map(|g| g.mesh.clone())
        .or_else(|| document.local_geometry(body).map(|(m, _)| m))
}

fn bounds(mesh: &TriMesh) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(f64::from(p[k]));
            hi[k] = hi[k].max(f64::from(p[k]));
        }
    }
    (lo, hi)
}

fn diagonal(mesh: &TriMesh) -> f64 {
    let (lo, hi) = bounds(mesh);
    ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt()
}

/// The vertices of the start mesh at the points `marked` flags on the
/// mesh the body draws now.
fn marked_on_start(start: &TriMesh, drawn: &TriMesh, marked: &[bool]) -> Vec<u32> {
    let at: std::collections::HashSet<[u32; 3]> = drawn
        .positions
        .iter()
        .zip(marked)
        .filter(|(_, m)| **m)
        .map(|(p, _)| p.map(f32::to_bits))
        .collect();
    start
        .positions
        .iter()
        .enumerate()
        .filter(|(_, p)| at.contains(&p.map(f32::to_bits)))
        .map(|(i, _)| i as u32)
        .collect()
}

/// A region from a command's arguments: `box`, `ball`, or the scan's
/// marking.
fn region(
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    all: bool,
) -> Result<MeshRegion, CommandError> {
    let xyz = |v: &Value, name: &str| -> Result<[f64; 3], CommandError> {
        let c = |k: &str, i: usize| v.get(k).or_else(|| v.get(i)).and_then(Value::as_f64);
        match (c("x", 0), c("y", 1), c("z", 2)) {
            (Some(x), Some(y), Some(z)) => Ok([x, y, z]),
            _ => Err(CommandError::bad(name, "points are {x, y, z}")),
        }
    };
    if let Some(b) = a.0.get("box") {
        let (lo, hi) = (
            b.get("min").or_else(|| b.get(0)),
            b.get("max").or_else(|| b.get(1)),
        );
        let (Some(lo), Some(hi)) = (lo, hi) else {
            return Err(CommandError::bad(
                "box",
                "is {min = {x, y, z}, max = {x, y, z}}",
            ));
        };
        return Ok(MeshRegion::Box {
            min: xyz(lo, "box")?,
            max: xyz(hi, "box")?,
        });
    }
    if let Some(b) = a.0.get("ball") {
        let centre = xyz(b.get("centre").unwrap_or(&Value::Null), "ball")?;
        let radius = b
            .get("radius")
            .and_then(Value::as_f64)
            .ok_or_else(|| CommandError::bad("ball", "is {centre = {x, y, z}, radius = r}"))?;
        return Ok(MeshRegion::Ball { centre, radius });
    }
    if let Some(marking) = ctx.marking.as_ref().filter(|m| m.body == body)
        && marking.vertices.iter().any(|v| *v)
    {
        let start = start_mesh(ctx.document, body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        let (drawn, _) = ctx
            .document
            .local_geometry(body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        return Ok(MeshRegion::Vertices(marked_on_start(
            &start,
            &drawn,
            &marking.vertices,
        )));
    }
    if all {
        Ok(MeshRegion::All)
    } else {
        Err(CommandError::failed(
            "mark part of the scan, or give a box or a ball",
        ))
    }
}

/// A step added to `body`'s history, its start kept first.
fn add_step(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    name: &str,
    op: MeshOp,
) -> CommandResult {
    ctx.document.set_mesh_start(body, true);
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == KIND && n.body == Some(body))
        .count()
        + 1;
    let feature = MeshFeature {
        name: format!("{name} {number}"),
        scan: body,
        op,
        offset: 0.0,
    };
    let id = ctx
        .document
        .add_feature_in_body(feature.clone(), feature.name.clone(), Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!({"feature": id.0.to_string(), "name": feature.name}))
}

/// Triangles, open edges, signed volume and pieces of a mesh, its
/// coincident vertices taken as one.
pub fn inspect(mesh: &TriMesh) -> Value {
    let mut first: std::collections::HashMap<[u32; 3], usize> = Default::default();
    let welded: Vec<usize> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| *first.entry(p.map(f32::to_bits)).or_insert(i))
        .collect();
    let mut parent: Vec<usize> = (0..mesh.positions.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut edges: std::collections::HashMap<(usize, usize), u32> = Default::default();
    let mut volume = 0.0;
    let triangles = mesh.indices.as_chunks::<3>().0;
    for t in triangles {
        let [a, b, c] = t.map(|v| welded[v as usize]);
        for (u, v) in [(a, b), (b, c), (c, a)] {
            *edges.entry((u.min(v), u.max(v))).or_default() += 1;
            let (ru, rv) = (root(&mut parent, u), root(&mut parent, v));
            parent[ru] = rv;
        }
        let [p, q, r] = t.map(|v| mesh.positions[v as usize].map(f64::from));
        volume += (p[0] * (q[1] * r[2] - q[2] * r[1]) - p[1] * (q[0] * r[2] - q[2] * r[0])
            + p[2] * (q[0] * r[1] - q[1] * r[0]))
            / 6.0;
    }
    let pieces: std::collections::HashSet<usize> = triangles
        .iter()
        .map(|t| root(&mut parent, welded[t[0] as usize]))
        .collect();
    let (lo, hi) = bounds(mesh);
    json!({
        "triangles": triangles.len(),
        "open_edges": edges.values().filter(|n| **n == 1).count(),
        "volume": volume,
        "pieces": pieces.len(),
        "bounds": {"min": lo, "max": hi},
    })
}

impl core_document::Workbench for MeshWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(ID, "Mesh", "Edit a scan's mesh as a history of steps")
            .icon("workbench-mesh")
            .feature_kinds([KIND])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        for (id, label, icon) in [
            (ISLANDS, "Remove islands", "triangle"),
            (DELETE, "Delete marked", "delete"),
            (FLIP, "Flip normals", "sketch-reorient"),
            (CUT, "Cut by plane", "cut"),
        ] {
            context.register_tool(ToolDescriptor::new_action(id, label, Some("mesh")).icon(icon));
        }
        let scan = |spec: CommandSpec| {
            spec.optional("scan", ParamKind::Id, "The mesh body; the selected one")
        };
        context.register_command(
            scan(CommandSpec::new(ISLANDS, "Remove the small loose pieces of a mesh"))
                .optional("smaller_than", ParamKind::Number, "Pieces whose box's diagonal is shorter than this go, mm; a hundredth of the mesh's")
                .returns("{feature, name}"),
        );
        context.register_command(
            scan(CommandSpec::new(DELETE, "Delete part of a mesh, the hole left open"))
                .optional("box", ParamKind::Any, "{min = {x, y, z}, max = {x, y, z}} in the body's frame")
                .optional("ball", ParamKind::Any, "{centre = {x, y, z}, radius = r} in the body's frame")
                .returns("{feature, name}")
                .note("Without a box or a ball it deletes the marked part of the scan. A face goes when all three of its corners are in the region."),
        );
        context.register_command(
            scan(CommandSpec::new(FLIP, "Turn a mesh's faces over"))
                .optional("box", ParamKind::Any, "Only these faces")
                .optional("ball", ParamKind::Any, "Only these faces")
                .returns("{feature, name}")
                .note("Without a region it turns the marked faces over, or every face when nothing is marked."),
        );
        context.register_command(
            scan(CommandSpec::new(CUT, "Cut a mesh by a plane, the cut closed flat"))
                .optional("plane", ParamKind::String, "xy, xz or yz, in the body's frame; xy")
                .optional("at", ParamKind::Number, "Where along the plane's normal, mm; the mesh's middle")
                .optional("keep", ParamKind::String, "below or above the plane; below")
                .optional("close", ParamKind::Bool, "Cap every loop of the cut flat; true")
                .optional("bridge", ParamKind::Number, "Gaps in the cut's rim up to this are closed straight, mm; a fiftieth of the mesh's diagonal")
                .returns("{feature, name}"),
        );
        context.register_command(
            scan(CommandSpec::new(INSPECT, "Count a mesh's triangles, open edges, pieces and volume"))
                .returns("{triangles, open_edges, volume, pieces, bounds = {min, max}}")
                .read_only()
                .note("Of the mesh the body draws now, its history applied. A closed mesh has no open edges; its volume is positive when its faces point out."),
        );
    }

    fn feature_info(&self, _node: &FeatureNode) -> FeatureInfo {
        FeatureInfo {
            icon: "workbench-mesh",
            kind_label: "Mesh step".to_string(),
            family_label: "Mesh edit".to_string(),
            builds_solid: false,
        }
    }

    fn parameters(&self, node: &FeatureNode) -> Vec<core_document::evaluate::Parameter> {
        use core_document::{evaluate::Parameter, expr::Dim};
        match MeshFeature::from_json(&node.data).map(|f| f.op) {
            Ok(MeshOp::RemoveIslands { .. }) => {
                vec![Parameter::new(
                    "smaller_than",
                    "Smaller than",
                    Dim::LENGTH,
                    "/op/RemoveIslands/smaller_than",
                )]
            }
            Ok(MeshOp::Cut { .. }) => vec![
                Parameter::new("offset", "Offset", Dim::LENGTH, "/offset"),
                Parameter::new("bridge", "Bridge gaps up to", Dim::LENGTH, "/op/Cut/bridge"),
            ],
            _ => Vec::new(),
        }
    }

    fn mesh_plans(&self, document: &Document) -> Vec<MeshPlan> {
        let mut by_body: std::collections::BTreeMap<uuid::Uuid, Vec<(u64, FeatureId, MeshOp)>> =
            Default::default();
        for (id, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != KIND || node.suppressed {
                continue;
            }
            let Some(f) = document
                .feature_values(*id)
                .and_then(|d| MeshFeature::from_json(d).ok())
            else {
                continue;
            };
            by_body
                .entry(f.scan.0)
                .or_default()
                .push((node.seq, *id, f.step()));
        }
        by_body
            .into_iter()
            .map(|(body, mut steps)| {
                steps.sort_by_key(|(seq, id, _)| (*seq, *id));
                // Steps after the body's tip are left out.
                let tip = document
                    .bodies()
                    .iter()
                    .find(|b| b.id.0 == body)
                    .and_then(|b| b.tip)
                    .and_then(|t| document.get_feature_meta(t))
                    .map(|n| n.seq);
                steps.retain(|(seq, _, _)| tip.is_none_or(|t| *seq <= t));
                MeshPlan {
                    body: BodyId(body),
                    features: steps.iter().map(|(_, id, _)| *id).collect(),
                    ops: steps.into_iter().map(|(_, _, op)| op).collect(),
                }
            })
            .collect()
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let Some(body) = ctx.selected_body_id.map(BodyId) else {
            return false;
        };
        let mesh = ctx.document.is_mesh_body(body) || ctx.document.mesh_start(body).is_some();
        match tool_id {
            DELETE => {
                mesh && ctx
                    .marking
                    .as_ref()
                    .is_some_and(|m| m.body == body && m.vertices.iter().any(|v| *v))
            }
            ISLANDS | FLIP | CUT => mesh,
            _ => false,
        }
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if let (WorkbenchInputEvent::ToolActivated, Some(tool @ (ISLANDS | DELETE | FLIP | CUT))) =
            (event, active_tool)
        {
            let args = CommandArgs::new();
            match self.run_command(tool, &args, ctx) {
                Ok(answer) => ctx.record(tool, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        InputResult::ignored()
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let a = Args(args);
        let body = target(&a, ctx)?;
        let start = start_mesh(ctx.document, body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        match id {
            ISLANDS => {
                let smaller_than = a
                    .opt_number("smaller_than")?
                    .unwrap_or(diagonal(&start) / 100.0);
                add_step(
                    ctx,
                    body,
                    "Remove islands",
                    MeshOp::RemoveIslands { smaller_than },
                )
            }
            DELETE => {
                let region = region(&a, ctx, body, false)?;
                add_step(ctx, body, "Delete", MeshOp::Delete { region })
            }
            FLIP => {
                let region = region(&a, ctx, body, true)?;
                add_step(ctx, body, "Flip", MeshOp::Flip { region })
            }
            CUT => {
                let normal = match a.opt_string("plane")?.unwrap_or("xy") {
                    "xy" | "XY" => [0.0, 0.0, 1.0],
                    "xz" | "XZ" => [0.0, 1.0, 0.0],
                    "yz" | "YZ" => [1.0, 0.0, 0.0],
                    _ => return Err(CommandError::bad("plane", "is xy, xz or yz")),
                };
                let (lo, hi) = bounds(&start);
                let mut origin: [f64; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
                if let Some(at) = a.opt_number("at")? {
                    for k in 0..3 {
                        if normal[k] != 0.0 {
                            origin[k] = at;
                        }
                    }
                }
                let keep_above = match a.opt_string("keep")?.unwrap_or("below") {
                    "below" => false,
                    "above" => true,
                    _ => return Err(CommandError::bad("keep", "is below or above")),
                };
                add_step(
                    ctx,
                    body,
                    "Cut",
                    MeshOp::Cut {
                        origin,
                        normal,
                        keep_above,
                        close: a.opt_bool("close")?.unwrap_or(true),
                        bridge: a.opt_number("bridge")?.unwrap_or(diagonal(&start) / 50.0),
                    },
                )
            }
            INSPECT => {
                let (drawn, _) = ctx
                    .document
                    .local_geometry(body)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                Ok(inspect(&drawn))
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Workbench;

    /// Every tool names an icon the set has. Marked points on the mesh the
    /// body draws name the start's vertices at the same spots, and a closed
    /// tetrahedron reads closed with its volume and one piece.
    #[test]
    fn the_bench_reads_its_meshes() {
        let mut context = WorkbenchContext::default();
        MeshWorkbench.configure(&mut context);
        for tool in context.tools() {
            let icon = tool.icon.expect("an icon");
            assert!(ui_kit::icon::exists(icon), "{icon}");
        }
        let corners = [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let start = TriMesh {
            positions: corners.to_vec(),
            indices: vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
            ..TriMesh::default()
        };
        let read = inspect(&start);
        assert_eq!(
            (read["open_edges"].clone(), read["pieces"].clone()),
            (json!(0), json!(1))
        );
        assert!((read["volume"].as_f64().unwrap() - 1.0 / 6.0).abs() < 1e-9);
        // The drawn mesh splits its corners, one copy per face.
        let drawn = TriMesh {
            positions: [0, 2, 1, 0, 1, 3].map(|i| corners[i]).to_vec(),
            ..TriMesh::default()
        };
        let marked = [false, true, false, false, false, true];
        assert_eq!(marked_on_start(&start, &drawn, &marked), vec![2, 3]);
        let cut = MeshFeature {
            name: "Cut 1".into(),
            scan: BodyId(uuid::Uuid::nil()),
            op: MeshOp::Cut {
                origin: [0.0, 0.0, 1.0],
                normal: [0.0, 0.0, 2.0],
                keep_above: false,
                close: true,
                bridge: 0.0,
            },
            offset: 0.5,
        };
        assert!(matches!(
            cut.step(),
            MeshOp::Cut {
                origin: [0.0, 0.0, 1.5],
                ..
            }
        ));
    }
}
