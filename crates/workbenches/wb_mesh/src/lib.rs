//! The Mesh bench: a mesh body's edits as a history of features on it,
//! each one step (`kernel_api::MeshOp`) the host builds from the body's
//! mesh start on a thread, in order, the result the body's mesh. Every
//! edit is undoable, editable and saved; the history's start is the mesh
//! as imported.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureId,
    FeatureInfo, FeatureNode, InputResult, MeshPlan, ParamKind, ToolDescriptor, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use kernel_api::{MeshOp, MeshRegion, TriMesh};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod solids;
mod steps;
mod task;

pub const ID: &str = "wb.mesh";
/// The feature kind of every mesh step.
pub const KIND: &str = "wb.mesh";

const ISLANDS: &str = "mesh.islands";
const DELETE: &str = "mesh.delete";
const FLIP: &str = "mesh.flip";
const CUT: &str = "mesh.cut";
const INSPECT: &str = "mesh.inspect";
const DEVIATION: &str = "mesh.deviation";

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

/// Each edited body's deviation from its start, by what it was measured
/// from, and those being measured.
#[derive(Default)]
struct Deviations {
    made: HashMap<BodyId, (u64, Arc<[f32]>)>,
    running: HashMap<BodyId, u64>,
}

#[derive(Default)]
pub struct MeshWorkbench {
    /// The deviation of the edit is shown.
    deviation: bool,
    maps: Arc<Mutex<Deviations>>,
    task: Option<task::Task>,
}

/// How far each point of the edited mesh lies from its start, signed;
/// unmeasured where it is a corner of surface an edit added (a cap, a
/// bridge): a triangle whose middle lies farther than a two-hundredth of
/// the diagonal from the start, which a cap's points on its rim alone
/// would not show.
pub fn deviation(
    kernel: &dyn kernel_api::KernelQueries,
    start: &TriMesh,
    now: &TriMesh,
) -> Result<Vec<f32>, String> {
    let added = diagonal(start) / 200.0;
    let triangles = now.indices.as_chunks::<3>().0;
    let middles = triangles.iter().map(|t| {
        let [a, b, c] = t.map(|v| now.positions[v as usize].map(f64::from));
        std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0)
    });
    let request = kernel_api::DistanceRequest {
        points: now
            .positions
            .iter()
            .map(|p| p.map(f64::from))
            .chain(middles)
            .collect(),
        normals: Vec::new(),
        max: Some(diagonal(start) / 20.0),
        facing_limit_deg: None,
    };
    let d = kernel
        .signed_distance(kernel_api::DistanceTarget::Mesh(start), &request)
        .map_err(|e| e.to_string())?;
    let (at_points, at_middles) = d.split_at(now.positions.len());
    let mut values: Vec<f32> = at_points.iter().map(|v| *v as f32).collect();
    for (t, m) in triangles.iter().zip(at_middles) {
        if !m.is_finite() || m.abs() > added {
            for v in t {
                values[*v as usize] = f32::NAN;
            }
        }
    }
    Ok(values)
}

/// What a deviation map reads: the largest and the RMS distance, the
/// 99th percentile, and the share of points an edit added.
pub fn deviation_reading(values: &[f32]) -> Value {
    let measured: Vec<f64> = values
        .iter()
        .filter(|v| !v.is_nan())
        .map(|v| f64::from(*v))
        .collect();
    let abs: Vec<f64> = measured.iter().map(|v| v.abs()).collect();
    let rms = if measured.is_empty() {
        0.0
    } else {
        (measured.iter().map(|v| v * v).sum::<f64>() / measured.len() as f64).sqrt()
    };
    json!({
        "max": abs.iter().copied().fold(0.0, f64::max),
        "rms": rms,
        "p99": metrology::stats::percentile(&abs, 0.99).unwrap_or(0.0),
        "added": if values.is_empty() { 0.0 } else { (values.len() - measured.len()) as f64 / values.len() as f64 },
    })
}

/// What a body's deviation is measured from: its mesh now and its start.
fn deviation_key(document: &Document, body: BodyId) -> Option<u64> {
    let start = document.mesh_start(body)?;
    let now = document.imported_geometry(body)?;
    Some(now.revision ^ start.revision.rotate_left(32))
}

/// The scan a command or tool works on: `scan` given, else the selected
/// mesh body.
fn target(a: &Args, ctx: &WorkbenchRuntimeContext) -> Result<BodyId, CommandError> {
    let body = match a.opt_body(ctx.document, "scan")? {
        Some(body) => body,
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
    if let Some(r) = a.0.get("region") {
        return serde_json::from_value(r.clone())
            .map_err(|e| CommandError::bad("region", e.to_string()));
    }
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
        let plan = MeshWorkbench::default()
            .mesh_plans(ctx.document)
            .into_iter()
            .find(|p| p.body == body);
        let vertices = if let Some(sources) =
            plan.as_ref().and_then(|p| ctx.document.mesh_sources(p))
        {
            if sources
                .iter()
                .zip(marking.vertices.iter())
                .any(|(source, marked)| *marked && source.is_none())
            {
                return Err(CommandError::failed(
                    "the marking includes added or reduced vertices without start identities; give a box or ball",
                ));
            }
            sources
                .iter()
                .zip(marking.vertices.iter())
                .filter_map(|(s, marked)| marked.then_some(*s).flatten())
                .collect()
        } else {
            marked_on_start(&start, &drawn, &marking.vertices)
        };
        if vertices.is_empty() {
            return Err(CommandError::failed(
                "the marking contains no surviving start vertices; give a box or ball",
            ));
        }
        return Ok(MeshRegion::Vertices(vertices));
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
    let feature = MeshFeature {
        name: ctx
            .document
            .new_object_name(name, core_document::Numbering::Counted),
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
        steps::register(context);
        let category = |id: &str| match id {
            "mesh.fill" | "mesh.smooth" => "repair",
            "mesh.offset" | "mesh.reduce" | "mesh.boolean" | "mesh.thicken" => "shape",
            DEVIATION => core_document::INSPECT_CATEGORY,
            _ => "clean",
        };
        for (id, label, icon) in [
            (ISLANDS, "Remove islands", "triangle"),
            (DELETE, "Delete marked", "delete"),
            (FLIP, "Flip normals", "sketch-reorient"),
            (CUT, "Cut by plane", "cut"),
        ]
        .into_iter()
        .chain(steps::TOOLS)
        .chain([(DEVIATION, "Deviation of the edit", "measure")])
        {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some(category(id))).icon(icon),
            );
        }
        let scan = |spec: CommandSpec| {
            spec.optional("scan", ParamKind::Id, "The mesh body; the selected one")
        };
        context.register_command(
            scan(CommandSpec::new(ISLANDS, "Remove specks"))
                .optional("smaller_than", ParamKind::Number, "Pieces whose box's diagonal is shorter than this go, mm; a hundredth of the mesh's")
                .returns("{feature, name}"),
        );
        context.register_command(
            scan(CommandSpec::new(DELETE, "Delete marked faces"))
                .optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
                .optional("box", ParamKind::Any, "{min = {x, y, z}, max = {x, y, z}} in the body's frame")
                .optional("ball", ParamKind::Any, "{centre = {x, y, z}, radius = r} in the body's frame")
                .returns("{feature, name}")
                .note("Without a box or a ball it deletes the marked part of the scan. A face goes when all three of its corners are in the region."),
        );
        context.register_command(
            scan(CommandSpec::new(FLIP, "Flip normals"))
                .optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
                .optional("box", ParamKind::Any, "Only these faces")
                .optional("ball", ParamKind::Any, "Only these faces")
                .returns("{feature, name}")
                .note("Without a region it turns the marked faces over, or every face when nothing is marked."),
        );
        context.register_command(
            scan(CommandSpec::new(CUT, "Cut by a plane"))
                .optional("plane", ParamKind::String, "xy, xz or yz, in the body's frame; xy")
                .optional("at", ParamKind::Number, "Where along the plane's normal, mm; the mesh's middle")
                .optional("keep", ParamKind::String, "below or above the plane; below")
                .optional("close", ParamKind::Bool, "Cap every loop of the cut flat; true")
                .optional("bridge", ParamKind::Number, "Gaps in the cut's rim up to this are closed straight, mm; a fiftieth of the mesh's diagonal")
                .returns("{feature, name}"),
        );
        context.register_command(
            scan(CommandSpec::new(DEVIATION, "Edit deviation"))
                .returns("{max, rms, p99, added}")
                .read_only()
                .note(
                    "Each point of the mesh the scan draws now is measured to its mesh as \
                     imported: `max` and `rms` in mm, `p99` the 99th percentile, and `added` \
                     the share of points on surface an edit added (caps, bridges: a triangle \
                     whose middle lies off the start). The tool shows it as a map, added \
                     surface grey.",
                ),
        );
        context.register_command(
            scan(CommandSpec::new(INSPECT, "Topology and volume"))
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
            Ok(MeshOp::Fill { .. }) => vec![Parameter::new(
                "holes_up_to",
                "Holes up to",
                Dim::LENGTH,
                "/op/Fill/holes_up_to",
            )],
            Ok(MeshOp::Smooth { .. }) => vec![
                Parameter::new(
                    "bumps_up_to",
                    "Bumps up to",
                    Dim::LENGTH,
                    "/op/Smooth/bumps_up_to",
                ),
                Parameter::new(
                    "move_at_most",
                    "Move at most",
                    Dim::LENGTH,
                    "/op/Smooth/move_at_most",
                ),
            ],
            Ok(MeshOp::Offset { .. }) => vec![Parameter::new(
                "distance",
                "Distance",
                Dim::LENGTH,
                "/op/Offset/distance",
            )],
            Ok(MeshOp::Reduce { .. }) => vec![Parameter::new(
                "tolerance",
                "Tolerance",
                Dim::LENGTH,
                "/op/Reduce/tolerance",
            )],
            _ => Vec::new(),
        }
    }

    fn mesh_plans(&self, document: &Document) -> Vec<MeshPlan> {
        let mut by_body: std::collections::BTreeMap<uuid::Uuid, Vec<(u64, FeatureId, MeshOp)>> =
            Default::default();
        for (id, node) in document
            .feature_tree()
            .ordered_nodes(|node| node.workbench_id.as_str() == KIND && !node.suppressed)
        {
            let Some(f) = document.feature_as::<MeshFeature>(*id) else {
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
                // Steps after the body's tip are left out.
                let tip = document
                    .body(BodyId(body))
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
            ISLANDS | FLIP | CUT | "mesh.fill" | "mesh.smooth" | "mesh.offset" | "mesh.reduce"
            | "mesh.boolean" | "mesh.thicken" => mesh && self.task.is_none(),
            DEVIATION => true,
            _ => false,
        }
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        let t = self.task.as_ref()?;
        let node = ctx.document.get_feature_meta(t.id)?;
        Some(core_document::TaskInfo {
            title: node.name.clone(),
            icon: "workbench-mesh",
            confirmable: true,
            stepwise: false,
            ..Default::default()
        })
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.task.as_ref().map(|t| t.id)
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        self.open_edit(ctx, id);
    }

    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        self.task
            .take()
            .map(|t| Box::new(t) as Box<dyn std::any::Any + Send>)
    }

    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        self.task = state
            .and_then(|s| s.downcast::<task::Task>().ok())
            .map(|s| *s);
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.close_task(
            ctx,
            core_document::TaskRequest {
                cancel: true,
                accept: false,
            },
        );
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        let widgets = self.task_widgets(ctx);
        let out = core_document::panel::show(ui, egui::Id::new("mesh-task"), &widgets, None);
        for event in out.events {
            self.task_event(ctx, &event);
        }
        self.close_task(ctx, request)
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        tool_id == DEVIATION && self.deviation
    }

    fn busy(&self) -> bool {
        self.maps.lock().is_ok_and(|m| !m.running.is_empty())
    }

    fn scalar_maps(&self, ctx: &WorkbenchRuntimeContext) -> Vec<core_document::ScalarMap> {
        let (true, Some(kernel)) = (self.deviation, ctx.kernel) else {
            return Vec::new();
        };
        let document = &*ctx.document;
        let mut out = Vec::new();
        for body in document.bodies().iter().map(|b| b.id) {
            let (Some(key), Some(start), Some((now, _))) = (
                deviation_key(document, body),
                document.mesh_start(body),
                document.local_geometry(body),
            ) else {
                continue;
            };
            let Ok(mut maps) = self.maps.lock() else {
                continue;
            };
            let made = maps
                .made
                .get(&body)
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| v.clone());
            let Some(values) = made else {
                if maps.running.get(&body) != Some(&key) {
                    maps.running.insert(body, key);
                    let (shared, start) = (Arc::clone(&self.maps), Arc::clone(&start.mesh));
                    std::thread::spawn(move || {
                        let values: Arc<[f32]> =
                            deviation(kernel, &start, &now).unwrap_or_default().into();
                        if let Ok(mut maps) = shared.lock() {
                            if maps.running.get(&body) == Some(&key) {
                                maps.running.remove(&body);
                            }
                            maps.made.insert(body, (key, values));
                        }
                    });
                }
                continue;
            };
            if values.is_empty() {
                continue;
            }
            let reading = deviation_reading(&values);
            let range = (reading["p99"].as_f64().unwrap_or(0.0) as f32).max(1e-3);
            let ramp = metrology::ramp::Ramp::Jet;
            let (under, over) = ramp.caps();
            let mm = |k: &str| {
                core_document::units::show_mm(
                    reading[k].as_f64().unwrap_or(0.0),
                    document.display_unit(),
                )
            };
            out.push(core_document::ScalarMap {
                body,
                revision: key,
                values,
                range: [-range, range],
                bands: 0,
                ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
                under,
                over,
                none: ctx.sketch_palette.inactive,
                legend: core_document::MapLegend {
                    title: "Deviation of the edit".to_string(),
                    unit: "mm".to_string(),
                    stats: vec![
                        ("Max".to_string(), mm("max")),
                        ("RMS".to_string(), mm("rms")),
                        (
                            "Added".to_string(),
                            format!("{:.1} %", reading["added"].as_f64().unwrap_or(0.0) * 100.0),
                        ),
                    ],
                    histogram: Vec::new(),
                    tolerance: None,
                },
            });
        }
        out
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if self.pick_rim(event, ctx) {
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(DEVIATION)) = (event, active_tool) {
            self.deviation = !self.deviation;
            return InputResult::consumed();
        }
        if let (
            WorkbenchInputEvent::ToolActivated,
            Some(
                tool @ (ISLANDS | DELETE | FLIP | CUT | "mesh.fill" | "mesh.smooth" | "mesh.offset"
                | "mesh.reduce" | "mesh.boolean" | "mesh.thicken"),
            ),
        ) = (event, active_tool)
        {
            if self.task.is_some() {
                return InputResult::consumed();
            }
            let args = CommandArgs::new();
            let had_start = ctx
                .selected_body_id
                .is_some_and(|id| ctx.document.mesh_start(BodyId(id)).is_some());
            match self.run_command(tool, &args, ctx) {
                Ok(answer) => self.open_created(ctx, tool, args, answer, had_start),
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
        if id == "mesh.set" || id == "mesh.result" {
            return self.step_command(id, &a, ctx);
        }
        let body = target(&a, ctx)?;
        let start = start_mesh(ctx.document, body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        if let Some((_, label, _)) = steps::TOOLS.iter().find(|(tool, _, _)| *tool == id) {
            let (mesh, _) = ctx
                .document
                .local_geometry(body)
                .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
            let op = steps::make(id, &a, ctx, body, &mesh)?;
            return add_step(ctx, body, label, op);
        }
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
                let (now, _) = ctx
                    .document
                    .local_geometry(body)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                let op = solids::cut(&a, &now)?;
                add_step(ctx, body, "Cut", op)
            }
            "mesh.preview_cut" => solids::preview_cut(&a, ctx, body),
            "mesh.export" => solids::export(&a, ctx, body),
            "mesh.trim_to_scan" => solids::trim(&a, ctx, body),
            DEVIATION => {
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

    #[test]
    fn task_fields_edit_record_cancel_and_show_results() {
        use bench_api::{PanelEvent, Widget};
        use core_document::{HookOutcome, ImportedGeometry, TaskRequest};
        for (tool, pointer, value) in [
            ("mesh.fill", "/op/Fill/holes_up_to", 2.0),
            ("mesh.smooth", "/op/Smooth/move_at_most", 0.02),
            ("mesh.offset", "/op/Offset/distance", -0.1),
            ("mesh.reduce", "/op/Reduce/triangle_limit", 100.0),
            ("mesh.thicken", "/op/Shell/thickness", 0.5),
        ] {
            let mut doc = Document::new("mesh task");
            let body = doc.create_body(None);
            let mesh = TriMesh {
                positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                indices: vec![0, 1, 2],
                ..TriMesh::default()
            };
            let asset = doc.add_asset(core_document::AssetReference::new(
                "assets/scan.stl",
                core_document::AssetType::Stl,
                json!({}),
            ));
            doc.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh: Arc::new(mesh.clone()),
                    mesh_path: None,
                    source_asset: Some(asset),
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
            ctx.selected_body_id = Some(body.0);
            let mut wb = MeshWorkbench::default();
            wb.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), &mut ctx);
            let id = wb.editing_feature().unwrap();
            assert!(
                wb.task_widgets(&ctx)
                    .iter()
                    .any(|w| w.id() == Some(pointer))
            );
            let initial = ctx.document.get_feature_meta(id).unwrap().data.clone();
            if tool == "mesh.smooth" {
                assert!(initial["op"]["Smooth"]["bumps_up_to"].as_f64().unwrap() >= 2.5);
                assert_eq!(initial["op"]["Smooth"]["keep_edges_deg"], json!(30.0));
            }
            let state = wb.suspend_session();
            assert!(wb.editing_feature().is_none());
            wb.resume_session(state);
            wb.task_event(
                &mut ctx,
                &PanelEvent::Number {
                    id: pointer.into(),
                    value,
                },
            );
            assert_eq!(
                ctx.document
                    .get_feature_meta(id)
                    .unwrap()
                    .data
                    .pointer(pointer)
                    .unwrap()
                    .as_f64(),
                Some(value)
            );
            if tool == "mesh.fill" {
                wb.task_event(
                    &mut ctx,
                    &PanelEvent::Choice {
                        id: "fairing".into(),
                        index: 1,
                    },
                );
                wb.task_event(&mut ctx, &PanelEvent::Pick { id: "rim".into() });
                ctx.hovered_body_id = Some(body.0);
                ctx.hovered_world_pos = Some([0.5, 0., 0.]);
                assert!(wb.pick_rim(
                    &WorkbenchInputEvent::MousePress {
                        button: core_document::MouseButton::Left,
                        viewport_pos: (50., 50.)
                    },
                    &mut ctx
                ));
            }
            let plan = wb.mesh_plans(ctx.document).remove(0);
            ctx.document.take_pending_ops();
            ctx.document.store_mesh_build(
                plan.clone(),
                kernel_api::MeshChainAnswer {
                    mesh: mesh.clone(),
                    sources: vec![Some(0), Some(1), Some(2)],
                    steps: vec![kernel_api::MeshStep {
                        faces: 1,
                        note: "measured".into(),
                        report: Some(kernel_api::MeshStepReport::Smooth {
                            max: 0.01,
                            rms: 0.005,
                        }),
                    }],
                    failed: None,
                },
            );
            assert!(
                ctx.document.take_pending_ops().is_empty(),
                "reports are derived"
            );
            assert!(
                wb.task_widgets(&ctx)
                    .iter()
                    .any(|w| matches!(w, Widget::Value { label, .. } if label == "RMS movement"))
            );
            let args = json!({"feature":id.0}).as_object().unwrap().clone();
            assert_eq!(
                wb.run_command("mesh.result", &args, &mut ctx).unwrap()["faces"],
                json!(1)
            );
            wb.close_task(
                &mut ctx,
                TaskRequest {
                    accept: true,
                    cancel: false,
                },
            );
            let calls = HookOutcome::take(&mut ctx).recorded;
            assert_eq!(calls.len(), 2);
            wb.open_edit(&ctx, id);
            let before = ctx.document.get_feature_meta(id).unwrap().data.clone();
            wb.task_event(
                &mut ctx,
                &PanelEvent::Number {
                    id: pointer.into(),
                    value: 3.0,
                },
            );
            assert!(
                wb.run_command("mesh.result", &args, &mut ctx).is_err(),
                "stale measurements hidden"
            );
            wb.close_task(
                &mut ctx,
                TaskRequest {
                    accept: false,
                    cancel: true,
                },
            );
            assert_eq!(ctx.document.get_feature_meta(id).unwrap().data, before);
            ctx.document.remove_feature(id).unwrap();
            wb.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), &mut ctx);
            let cancelled = wb.editing_feature().unwrap();
            wb.close_task(
                &mut ctx,
                TaskRequest {
                    accept: false,
                    cancel: true,
                },
            );
            assert!(ctx.document.get_feature_meta(cancelled).is_none());
        }
    }

    /// Every tool names an icon the set has. Marked points on the mesh the
    /// body draws name the start's vertices at the same spots, a closed
    /// tetrahedron reads closed with its volume and one piece, and a
    /// deviation reads its extremes.
    #[test]
    fn the_bench_reads_its_meshes() {
        let mut context = WorkbenchContext::default();
        MeshWorkbench::default().configure(&mut context);
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
        // A deviation map reads its largest and RMS distance and the share
        // of points an edit added.
        let read = deviation_reading(&[0.1, -0.2, f32::NAN, 0.0]);
        assert!((read["max"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!((read["added"].as_f64().unwrap() - 0.25).abs() < 1e-12);
        assert!((read["rms"].as_f64().unwrap() - (0.05f64 / 3.0).sqrt()).abs() < 1e-6);
    }
}
