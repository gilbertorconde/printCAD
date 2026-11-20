//! The Mesh bench: a mesh body's edits as a history of features on it,
//! each one step (`kernel_api::MeshOp`) the host builds from the body's
//! mesh start on a thread, in order, the result the body's mesh. Every
//! edit is undoable, editable and saved; the history's start is the mesh
//! as imported.

// Without the egui feature the crate is the headless subset its dependents'
// tests and the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code, unused_imports))]

use core_document::{
    Args, BenchBuild, BenchCommands, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec,
    CommandTable, Document, FeatureId, FeatureInfo, FeatureNode, InputResult, MeshPlan, ParamKind,
    ToolDescriptor, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId,
    WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use kernel_api::{MeshOp, MeshRegion, TriMesh};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

mod commands;
mod crossings;
mod pieces;
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
const SPLIT: &str = "mesh.split";
const CROSSINGS: &str = "mesh.intersections";
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

#[derive(Default)]
pub struct MeshWorkbench {
    tab: Tab,
}

/// What the bench holds for the document on screen, moved with its tab.
#[derive(Default)]
struct Tab {
    /// The deviation of the edit is shown.
    deviation: bool,
    /// Each edited body's deviation from its start, by what it was
    /// measured from.
    deviations: core_document::jobs::Keyed<BodyId, Arc<[f32]>>,
    /// Where the selected body's mesh crosses itself is shown.
    crossings: bool,
    /// Each body's crossing pairs of triangles, by the mesh they were
    /// found on.
    crossing_pairs: core_document::jobs::Keyed<BodyId, Arc<[[u32; 2]]>>,
    /// The body and mesh whose crossings were last reported.
    crossings_said: Option<(BodyId, u64)>,
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
/// 99th percentile, and the share of points an edit added. A value that
/// is not finite measured nothing: it counts as added.
pub fn deviation_reading(values: &[f32]) -> Value {
    let measured: Vec<f64> = values
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| f64::from(*v))
        .collect();
    let abs: Vec<f64> = measured.iter().map(|v| v.abs()).collect();
    let summary = metrology::stats::summary(&abs);
    json!({
        "max": summary.map_or(0.0, |s| s.max),
        "rms": summary.map_or(0.0, |s| s.rms),
        "p99": metrology::stats::percentile(&abs, 0.99).unwrap_or(0.0),
        "added": if values.is_empty() { 0.0 } else { (values.len() - measured.len()) as f64 / values.len() as f64 },
    })
}

/// What a body's deviation is measured from: its mesh now and its start.
fn deviation_key(document: &Document, body: BodyId) -> Option<u64> {
    let start = document.mesh_start(body)?;
    let now = document.derived().imported_geometry(body)?;
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
        .or_else(|| document.derived().local_geometry(body).map(|(m, _)| m))
}

/// The length of the mesh's box diagonal; zero for a mesh with no points.
fn diagonal(mesh: &TriMesh) -> f64 {
    let Some((lo, hi)) = mesh.bounds() else {
        return 0.0;
    };
    let d = |k: usize| f64::from(hi[k]) - f64::from(lo[k]);
    (d(0).powi(2) + d(1).powi(2) + d(2).powi(2)).sqrt()
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
    let xyz = |v: &Value, name: &str| core_document::command::vec3_of(Some(v), name);
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
    if let Some(marking) = ctx.current_marking().filter(|m| m.body == body)
        && marking.vertices.iter().any(|v| *v)
    {
        let start = start_mesh(ctx.document, body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        let (drawn, _) = ctx
            .document
            .derived()
            .local_geometry(body)
            .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
        let plan = MeshWorkbench::default()
            .mesh_plans(ctx.document)
            .into_iter()
            .find(|p| p.body == body);
        let vertices = if let Some(sources) = plan
            .as_ref()
            .and_then(|p| ctx.document.derived().mesh_sources(p))
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
    let bounds = mesh
        .bounds()
        .map(|(lo, hi)| json!({"min": lo.map(f64::from), "max": hi.map(f64::from)}));
    json!({
        "triangles": triangles.len(),
        "open_edges": edges.values().filter(|n| **n == 1).count(),
        "volume": volume,
        "pieces": pieces.len(),
        "bounds": bounds,
    })
}

impl core_document::Workbench for MeshWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(ID, "Mesh", "Edit a scan's mesh as a history of steps")
            .icon("workbench-mesh")
            .feature_kinds([KIND])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        let category = |id: &str| match id {
            "mesh.fill" | "mesh.smooth" | "mesh.repair_intersections" => "repair",
            "mesh.offset" | "mesh.reduce" | "mesh.remesh" | "mesh.subdivide" | "mesh.boolean"
            | "mesh.thicken" => "shape",
            DEVIATION | CROSSINGS => core_document::INSPECT_CATEGORY,
            _ => "clean",
        };
        for (id, label, icon) in [
            (ISLANDS, "Remove islands", "triangle"),
            (DELETE, "Delete marked", "delete"),
            (FLIP, "Flip normals", "sketch-reorient"),
            (CUT, "Cut by plane", "cut"),
            (SPLIT, "Split into bodies", "separate-pieces"),
        ]
        .into_iter()
        .chain(steps::TOOLS)
        .chain([
            (DEVIATION, "Deviation of the edit", "measure"),
            (CROSSINGS, "Self-intersections", "warning"),
        ]) {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some(category(id))).icon(icon),
            );
        }
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        Some(&mut self.tab)
    }

    fn busy(&self) -> bool {
        self.tab.deviations.busy() || self.tab.crossing_pairs.busy()
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        self.report_crossings(ctx);
    }
}

impl core_document::BenchFeatures for MeshWorkbench {
    fn feature_info(&self, _node: &FeatureNode) -> FeatureInfo {
        FeatureInfo {
            icon: "workbench-mesh",
            kind_label: "Mesh step".to_string(),
            family_label: "Mesh edit".to_string(),
            builds_solid: false,
        }
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.tab.task.as_ref().map(|t| t.id)
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        self.open_edit(ctx, id);
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
}

impl core_document::BenchTools for MeshWorkbench {
    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let Some(body) = ctx.selected_body_id.map(BodyId) else {
            return false;
        };
        let mesh = ctx.document.is_mesh_body(body) || ctx.document.mesh_start(body).is_some();
        match tool_id {
            DELETE => {
                mesh && ctx
                    .current_marking()
                    .is_some_and(|m| m.body == body && m.vertices.iter().any(|v| *v))
            }
            ISLANDS | FLIP | CUT | SPLIT => mesh && self.tab.task.is_none(),
            id if steps::is_tool(id) => mesh && self.tab.task.is_none(),
            DEVIATION => true,
            CROSSINGS => mesh,
            _ => false,
        }
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        (tool_id == DEVIATION && self.tab.deviation) || (tool_id == CROSSINGS && self.tab.crossings)
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            scalar_maps: [self.scalar_maps(ctx), self.crossing_maps(ctx)].concat(),
            ..Default::default()
        }
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
            self.tab.deviation = !self.tab.deviation;
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(CROSSINGS)) = (event, active_tool) {
            self.tab.crossings = !self.tab.crossings;
            self.tab.crossings_said = None;
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(SPLIT)) = (event, active_tool) {
            self.split_selected(ctx);
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(tool)) = (event, active_tool)
            && (matches!(tool, ISLANDS | DELETE | FLIP | CUT) || steps::is_tool(tool))
        {
            if self.tab.task.is_some() {
                return InputResult::consumed();
            }
            let args = CommandArgs::new();
            match self.run_command(tool, &args, ctx) {
                Ok(answer) => self.open_created(ctx, tool, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        InputResult::ignored()
    }
}

impl core_document::BenchPanels for MeshWorkbench {
    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        let t = self.tab.task.as_ref()?;
        let node = ctx.document.get_feature_meta(t.id)?;
        Some(core_document::TaskInfo {
            title: node.name.clone(),
            icon: "workbench-mesh",
            confirmable: true,
            stepwise: false,
            ..Default::default()
        })
    }
}

impl core_document::BenchBuild for MeshWorkbench {
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
}

impl core_document::BenchParameters for MeshWorkbench {
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
            Ok(MeshOp::Remesh { .. }) => vec![Parameter::new(
                "edge_length",
                "Edge length",
                Dim::LENGTH,
                "/op/Remesh/edge_length",
            )],
            _ => Vec::new(),
        }
    }
}

impl core_document::BenchOutput for MeshWorkbench {}

impl core_document::BenchCommands for MeshWorkbench {
    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        commands::run(self, id, args, ctx)
    }
}

#[cfg(feature = "egui")]
impl bench_panel::BenchUi for MeshWorkbench {
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        let widgets = self.task_widgets(ctx);
        let out = bench_panel::show(ui, egui::Id::new("mesh-task"), &widgets, None);
        for event in out.events {
            self.task_event(ctx, &event);
        }
        self.close_task(ctx, request)
    }
}

impl MeshWorkbench {
    /// The toolbar's Split into bodies on the selected mesh body: recorded
    /// as the call and one step of undo.
    fn split_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.tab.task.is_some() {
            return;
        }
        let Some(body) = ctx.selected_body_id.map(BodyId) else {
            return;
        };
        match pieces::split(ctx, body) {
            Ok(made) => {
                let args = core_document::command::object(json!({"scan": body.0.to_string()}));
                ctx.record(SPLIT, args, pieces::answer(&made));
                ctx.request(core_document::HostRequest::JournalLabel(
                    "Split into bodies".into(),
                ));
                ctx.log_info(format!("Split into {} bodies", made.len()));
            }
            Err(why) => ctx.log_warn(format!("Split into bodies: {why}")),
        }
    }

    fn scalar_maps(&self, ctx: &WorkbenchRuntimeContext) -> Vec<core_document::ScalarMap> {
        let (true, Some(kernel)) = (self.tab.deviation, ctx.kernel) else {
            return Vec::new();
        };
        let document = &*ctx.document;
        let mut out = Vec::new();
        for body in document.bodies().iter().map(|b| b.id) {
            let (Some(key), Some(start), Some((now, _))) = (
                deviation_key(document, body),
                document.mesh_start(body),
                document.derived().local_geometry(body),
            ) else {
                continue;
            };
            let start = Arc::clone(&start.mesh);
            let measured = self.tab.deviations.ensure(body, key, move || {
                Ok(deviation(kernel, &start, &now).unwrap_or_default().into())
            });
            let Some(values) = measured else {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{BenchBuild, BenchCommands, BenchFeatures, BenchTools, Workbench};

    #[test]
    fn every_section_of_the_mesh_prose_names_a_command() {
        let bench = &mut crate::MeshWorkbench::default();
        for prose in [
            crate::commands::prose(),
            crate::steps::prose(),
            crate::solids::prose(),
        ] {
            core_document::testing::assert_prose_names_commands(bench, prose);
        }
    }

    #[test]
    fn a_cut_takes_any_plane_and_keeps_the_named_ones_as_shorthand() {
        let cube = TriMesh {
            positions: vec![[0., 0., 0.], [10., 10., 10.], [0., 10., 0.]],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        let plane = |args: serde_json::Value| {
            let args = core_document::command::object(args);
            match solids::cut(&Args(&args), &cube) {
                Ok(MeshOp::Cut { origin, normal, .. }) => Ok((origin, normal)),
                Ok(other) => panic!("{other:?}"),
                Err(e) => Err(e.to_string()),
            }
        };
        let named = plane(json!({"plane": "xz", "at": 3})).unwrap();
        assert_eq!(named, ([5., 3., 5.], [0., 1., 0.]));
        assert_eq!(
            plane(json!({"normal": {"x": 0, "y": 2, "z": 0}, "at": 3})),
            Ok(named)
        );
        let (origin, normal) = plane(json!({"normal": [1, 1, 0], "origin": [1, 2, 3]})).unwrap();
        assert_eq!(origin, [1., 2., 3.]);
        let half = 0.5f64.sqrt();
        assert!((normal[0] - half).abs() < 1e-12 && (normal[1] - half).abs() < 1e-12);
        assert_eq!(normal[2], 0.0);
        for refused in [
            json!({"plane": "xy", "normal": [0, 0, 1]}),
            json!({"normal": [0, 0, 0]}),
            json!({"origin": [0, 0, 0], "at": 1}),
        ] {
            assert!(plane(refused.clone()).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_diagonal_spans_the_points_and_an_empty_mesh_has_none() {
        assert_eq!(diagonal(&TriMesh::default()), 0.0);
        let mesh = TriMesh {
            positions: vec![[1., 1., 1.], [4., 5., 13.], [2., 2., 2.]],
            ..TriMesh::default()
        };
        assert_eq!(diagonal(&mesh), 13.0);
    }

    #[test]
    fn task_fields_edit_record_cancel_and_show_results() {
        use bench_api::{PanelEvent, Widget};
        use core_document::{HookOutcome, ImportedGeometry, TaskRequest};
        for (tool, pointer, value) in [
            ("mesh.fill", "/op/Fill/holes_up_to", 2.0),
            ("mesh.smooth", "/op/Smooth/move_at_most", 0.02),
            ("mesh.offset", "/op/Offset/distance", -0.1),
            ("mesh.reduce", "/op/Reduce/triangle_limit", 100.0),
            ("mesh.remesh", "/op/Remesh/edge_length", 0.5),
            ("mesh.subdivide", "/op/Subdivide/levels", 2.0),
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
            doc.derived_mut().set_imported_geometry(
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
            ctx.document.derived_mut().store_mesh_build(
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
            let cancel = TaskRequest {
                accept: false,
                cancel: true,
            };
            core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
                let mut ctx = WorkbenchRuntimeContext::new(doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
                wb.open_edit(&ctx, id);
                let edit = PanelEvent::Number {
                    id: pointer.into(),
                    value: 3.0,
                };
                wb.task_event(&mut ctx, &edit);
                assert!(
                    wb.run_command("mesh.result", &args, &mut ctx).is_err(),
                    "stale measurements hidden"
                );
                wb.close_task(&mut ctx, cancel);
            });
            // A scan's first step sets its start, which Cancel takes back.
            doc.remove_feature(id).unwrap();
            doc.set_mesh_start(body, false);
            core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
                let mut ctx = WorkbenchRuntimeContext::new(doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
                ctx.selected_body_id = Some(body.0);
                wb.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), &mut ctx);
                assert!(ctx.document.mesh_start(body).is_some());
                wb.close_task(&mut ctx, cancel);
            });
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
        // of points an edit added; an infinity measured nothing.
        let read = deviation_reading(&[0.1, -0.2, f32::NAN, 0.0, f32::INFINITY, f32::NEG_INFINITY]);
        assert!((read["max"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!((read["added"].as_f64().unwrap() - 0.5).abs() < 1e-12);
        assert!((read["rms"].as_f64().unwrap() - (0.05f64 / 3.0).sqrt()).abs() < 1e-6);
    }

    /// A closed box from `lo` to `hi`, wound outward, its corners its own.
    fn cube(lo: [f32; 3], hi: [f32; 3]) -> TriMesh {
        let positions = (0..8)
            .map(|i| std::array::from_fn(|k| if i >> k & 1 == 1 { hi[k] } else { lo[k] }))
            .collect();
        TriMesh {
            positions,
            indices: vec![
                0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2,
                4, 6, 1, 3, 5, 3, 7, 5,
            ],
            ..TriMesh::default()
        }
    }

    /// A mesh body named Scan holding `mesh`, as an import makes one.
    fn mesh_body(doc: &mut Document, mesh: TriMesh) -> BodyId {
        let body = doc.create_body(Some("Scan".into()));
        let asset = doc.add_asset(core_document::AssetReference::new(
            "assets/scan.stl",
            core_document::AssetType::Stl,
            json!({}),
        ));
        doc.derived_mut().set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh),
                mesh_path: None,
                source_asset: Some(asset),
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        body
    }

    /// A kernel that finds these pairs of triangles crossing in any mesh,
    /// or with none refuses to look.
    struct Crossing(Option<&'static [[u32; 2]]>);

    impl kernel_api::KernelQueries for Crossing {
        fn self_intersections(&self, _mesh: &TriMesh) -> kernel_api::KernelResult<Vec<[u32; 2]>> {
            self.0
                .map(<[_]>::to_vec)
                .ok_or_else(|| kernel_api::KernelError::Unsupported("not yet".into()))
        }

        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("project_edge".into()))
        }
    }

    static FINDS: Crossing = Crossing(Some(&[[0, 13], [1, 13]]));
    static REFUSES: Crossing = Crossing(None);

    /// The crossing map of the selected body, once its search is done.
    fn crossing_map(
        wb: &MeshWorkbench,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::ScalarMap> {
        for _ in 0..2000 {
            let map = wb
                .viewport(ctx)
                .scalar_maps
                .into_iter()
                .find(|m| m.legend.title == "Self-intersections");
            let failed = ctx.selected_body_id.map(BodyId).is_some_and(|b| {
                crossings::key(ctx.document, b)
                    .is_some_and(|k| wb.tab.crossing_pairs.failure(b, k).is_some())
            });
            if map.is_some() || failed {
                return map;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("the search never ended");
    }

    /// The kernel's crossing pairs come back from the command by count and
    /// triangle, and the map colours their corners alone; a kernel that
    /// cannot look turns the map off and the command fails.
    #[test]
    fn crossings_are_counted_and_their_corners_shown() {
        let mut both = cube([0.; 3], [10.; 3]);
        let other = cube([5.; 3], [15.; 3]);
        both.indices.extend(
            other
                .indices
                .iter()
                .map(|i| i + both.positions.len() as u32),
        );
        both.positions.extend(other.positions);
        let mut doc = Document::new("crossing");
        let body = mesh_body(&mut doc, both.clone());
        for (kernel, finds) in [(&FINDS, true), (&REFUSES, false)] {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
            ctx.selected_body_id = Some(body.0);
            ctx.kernel = Some(kernel);
            let mut wb = MeshWorkbench::default();
            let answer = wb.run_command(CROSSINGS, &CommandArgs::new(), &mut ctx);
            assert_eq!(answer.is_ok(), finds, "{answer:?}");
            if finds {
                assert_eq!(
                    answer.unwrap(),
                    json!({"pairs": 2, "triangles": [0, 1, 13]})
                );
            }
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some(CROSSINGS),
                &mut ctx,
            );
            assert!(wb.tool_toggled(CROSSINGS));
            let map = crossing_map(&wb, &ctx);
            wb.on_frame(0.0, &mut ctx);
            assert_eq!(wb.tool_toggled(CROSSINGS), finds, "a refusal turns it off");
            let Some(map) = map else {
                assert!(!finds, "no map from a kernel that finds");
                continue;
            };
            let corners: std::collections::BTreeSet<u32> = [0usize, 1, 13]
                .iter()
                .flat_map(|t| both.indices[t * 3..t * 3 + 3].to_vec())
                .collect();
            for (v, value) in map.values.iter().enumerate() {
                assert_eq!(
                    value.is_finite(),
                    corners.contains(&(v as u32)),
                    "vertex {v}"
                );
            }
        }
    }

    /// Two cubes in one mesh body split into two mesh bodies holding a
    /// cube each, named after it, placed and drawn as it was and shown
    /// in its place; a peer applying the ops sees the same, a save keeps
    /// them, and one undo gives the one body back.
    #[test]
    fn two_cubes_in_one_mesh_split_into_two_bodies() {
        let mut both = cube([0.; 3], [10.; 3]);
        let small = cube([20., 0., 0.], [25., 5., 5.]);
        both.indices.extend(
            small
                .indices
                .iter()
                .map(|i| i + both.positions.len() as u32),
        );
        both.positions.extend(small.positions);
        assert!((inspect(&both)["volume"].as_f64().unwrap() - 1125.0).abs() < 1e-6);
        let mut doc = Document::new("split");
        let body = mesh_body(&mut doc, both);
        let placement = core_document::BodyPlacement {
            translation: [1.0, 2.0, 3.0],
            ..Default::default()
        };
        doc.set_body_placement(body, placement);
        doc.set_body_shading(body, core_document::Shading::Sharp);
        doc.set_body_scan(body, Some(core_document::ScanInfo::default()));
        doc.take_pending_ops();
        let mut peer = doc.clone();
        let mut journal = core_document::history::OpJournal::new(16);
        journal.note(&mut doc);
        let before = doc.replicated_projection();

        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
        let args = core_document::command::object(json!({"scan": body.0.to_string()}));
        let answer = MeshWorkbench::default()
            .run_command(SPLIT, &args, &mut ctx)
            .unwrap();
        let made: Vec<BodyId> = answer["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| BodyId(uuid::Uuid::parse_str(b.as_str().unwrap()).unwrap()))
            .collect();
        let pieces = |doc: &Document| -> Vec<(String, f64, bool)> {
            made.iter()
                .map(|b| {
                    let entry = doc.body(*b).expect("the piece's body");
                    assert!(doc.is_mesh_body(*b));
                    assert_eq!(entry.placement, placement);
                    assert_eq!(entry.shading, core_document::Shading::Sharp);
                    assert!(entry.scan.is_some());
                    let (mesh, _) = doc.derived().local_geometry(*b).expect("the piece's mesh");
                    let read = inspect(&mesh);
                    assert_eq!(
                        (read["pieces"].clone(), read["open_edges"].clone()),
                        (json!(1), json!(0))
                    );
                    (
                        entry.name.clone(),
                        read["volume"].as_f64().unwrap(),
                        !entry.hidden,
                    )
                })
                .collect()
        };
        let want = [
            ("Scan 1".to_string(), 1000.0, true),
            ("Scan 2".to_string(), 125.0, true),
        ];
        let close = |got: Vec<(String, f64, bool)>| {
            assert_eq!(got.len(), want.len(), "{got:?}");
            for (g, w) in got.iter().zip(&want) {
                assert!(
                    g.0 == w.0 && (g.1 - w.1).abs() < 1e-6 && g.2 == w.2,
                    "{got:?}"
                );
            }
        };
        close(pieces(&doc));
        assert!(doc.body(body).unwrap().hidden, "the body split is hidden");

        for op in doc.take_pending_ops() {
            peer.apply_op(&op);
        }
        close(pieces(&peer));
        let saved = doc.save_to_bytes(core_document::Compression::None).unwrap();
        close(pieces(&Document::load_from_bytes(saved).unwrap()));

        journal.note(&mut doc);
        assert!(journal.undo(&mut doc).is_some());
        assert_eq!(
            doc.replicated_projection(),
            before,
            "undo gives the one body back"
        );
        assert!(made.iter().all(|b| doc.body(*b).is_none()));
        assert!(journal.redo(&mut doc).is_some());
        close(pieces(&doc));

        // A mesh of one piece has nothing to split.
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
        let one = core_document::command::object(json!({"scan": made[0].0.to_string()}));
        assert!(
            MeshWorkbench::default()
                .run_command(SPLIT, &one, &mut ctx)
                .is_err()
        );
    }

    /// A delete takes the marking only while the scan still has the mesh
    /// the marking numbers; one of an earlier mesh deletes nothing.
    #[test]
    fn a_delete_takes_only_a_marking_of_the_mesh_shown() {
        let mut doc = Document::new("marked");
        let body = doc.create_body(None);
        let asset = doc.add_asset(core_document::AssetReference::new(
            "assets/scan.stl",
            core_document::AssetType::Stl,
            json!({}),
        ));
        doc.derived_mut().set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: Arc::new(TriMesh {
                    positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
                    indices: vec![0, 1, 2, 1, 3, 2],
                    ..TriMesh::default()
                }),
                mesh_path: None,
                source_asset: Some(asset),
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let revision = doc.derived().imported_geometry(body).unwrap().revision;
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.; 3], [0.; 3], (0, 0, 100, 100));
        ctx.selected_body_id = Some(body.0);
        let mut wb = MeshWorkbench::default();
        for stale in [true, false] {
            ctx.marking = Some(core_document::MarkedSurface {
                body,
                revision: revision + u64::from(stale),
                vertices: Arc::new(vec![true, true, true, false]),
            });
            assert_eq!(wb.is_tool_enabled(DELETE, &ctx), !stale);
            let made = wb.run_command(DELETE, &CommandArgs::new(), &mut ctx);
            assert_eq!(made.is_ok(), !stale, "{made:?}");
            assert_eq!(wb.mesh_plans(ctx.document).len(), usize::from(!stale));
        }
        let ops = wb.mesh_plans(ctx.document).remove(0).ops;
        assert!(
            matches!(&ops[..], [MeshOp::Delete { region: MeshRegion::Vertices(v) }] if *v == [0, 1, 2]),
            "{ops:?}"
        );
    }
}
