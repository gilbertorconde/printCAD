//! `WasmWorkbench`: a package seen through the `Workbench` trait, so the
//! rest of the app cannot tell it from a built-in bench.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use bench_api::{Frame, Widget};
use core_document::rebuild::{BuildError, BuildPlan, RebuildJob};
use core_document::{
    ActionDescriptor, BodyId, Chord, CommandArgs, CommandError, CommandResult, CommandSpec,
    Document, FeatureId, FeatureInfo, FeatureNode, InputResult, MarkKind, MenuItem, MenuScope,
    OverlayMesh, Parameter, PropertyHints, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay,
    StatusItems, TaskInfo, ToolBehavior, ToolDescriptor, ToolHint, ViewportHud, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use serde::de::DeserializeOwned;

use crate::convert::{self, intern};
use crate::engine::Budget;
use crate::guest::{Aftermath, Guest, Loaded};
use crate::host::{Access, PackageInfo};
use crate::package::Package;

/// A loaded workbench package.
pub struct WasmWorkbench {
    descriptor: WorkbenchDescriptor,
    registration: bench_api::Registration,
    package: Arc<PackageInfo>,
    /// The package's icons, by the name it uses, as registered.
    icons: HashMap<String, &'static str>,
    inner: Mutex<Inner>,
}

struct Inner {
    guest: Guest,
    /// The bench's drawing as it last answered.
    frame: Frame,
    /// What the frame was drawn for; asked again when it changes.
    frame_key: Option<u64>,
    /// Something reached the bench since the frame was drawn.
    stale: bool,
    active: bool,
    info: HashMap<u64, FeatureInfo>,
    params: HashMap<u64, Vec<Parameter>>,
    settings_panel: Option<Vec<Widget>>,
}

/// Load `package`, allowing it `granted` of what it asks for.
pub fn load(package: &Package, granted: &bench_api::Capabilities) -> Result<WasmWorkbench, String> {
    let manifest = &package.manifest;
    let component =
        crate::engine::component(&package.wasm(), &package.compiled_dir(), &manifest.id)?;
    let info = Arc::new(PackageInfo {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        kinds: manifest.feature_kinds.clone(),
        granted: manifest.capabilities.and(granted),
        helpers: crate::jobs::helpers_dir(&package.dir),
    });
    let loaded = Arc::new(Loaded::new(
        component,
        info.clone(),
        package.data_dir(),
        manifest.memory_mb as usize * 1024 * 1024,
    )?);
    let mut guest = Guest::new(loaded)?;
    let registration: bench_api::Registration = guest
        .call(Budget::Long, Access::None, |b, s| b.call_describe(s))
        .map(|(json, _)| json)
        .ok_or_else(|| format!("{} did not describe itself", manifest.id))
        .and_then(|json| {
            serde_json::from_str(&json).map_err(|e| format!("{}'s description: {e}", manifest.id))
        })?;

    let mut icons = HashMap::new();
    for (name, svg) in package.icons() {
        let full = format!("{}/{name}", manifest.id);
        ui_kit::icon::register(&full, svg);
        icons.insert(name, intern(&full));
    }
    let icon_of = |name: &str| -> &'static str {
        match icons.get(name) {
            Some(full) => full,
            None if name.is_empty() => "workbench-print",
            None => intern(name),
        }
    };
    let mut descriptor = WorkbenchDescriptor::new(
        manifest.id.clone(),
        if registration.label.is_empty() {
            manifest.name.clone()
        } else {
            registration.label.clone()
        },
        if registration.description.is_empty() {
            manifest.description.clone()
        } else {
            registration.description.clone()
        },
    )
    .icon(icon_of(&registration.icon))
    .feature_kinds(manifest.feature_kinds.clone());
    if registration.modal {
        descriptor = descriptor.modal();
    }
    Ok(WasmWorkbench {
        descriptor,
        registration,
        package: info,
        icons,
        inner: Mutex::new(Inner {
            guest,
            frame: Frame::default(),
            frame_key: None,
            stale: true,
            active: false,
            info: HashMap::new(),
            params: HashMap::new(),
            settings_panel: None,
        }),
    })
}

/// Answers kept per feature revision; past this many the cache starts
/// over, so a long session of edits does not grow it without end.
const CACHE_LIMIT: usize = 4096;

fn parse<T: DeserializeOwned>(id: &str, what: &str, json: &str) -> Option<T> {
    match serde_json::from_str(json) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(target: "printcad.bench", package = %id, "{what} does not read: {e}");
            None
        }
    }
}

fn hash_of(parts: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Mark `body` for rebuilding after its history changed: its first
/// feature of the package, or, with no features left at all, its derived
/// solid goes.
pub(crate) fn invalidate_owned(document: &mut Document, package: &PackageInfo, body: BodyId) {
    let first = owned_in_body(document, package, body).into_iter().next();
    match first {
        Some(first) => document.mark_feature_dirty(first),
        None => {
            let any = document
                .feature_tree()
                .all_nodes()
                .any(|(_, n)| n.body == Some(body));
            if !any && !document.body_solid_is_imported(body) {
                document.remove_imported_geometry(body);
            }
        }
    }
}

/// The package's features in `body`, in history order.
fn owned_in_body(document: &Document, package: &PackageInfo, body: BodyId) -> Vec<FeatureId> {
    let mut nodes: Vec<(u64, FeatureId)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body) && package.owns(n.workbench_id.as_str()))
        .map(|(id, n)| (n.seq, *id))
        .collect();
    nodes.sort();
    nodes.into_iter().map(|(_, id)| id).collect()
}

impl WasmWorkbench {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn id(&self) -> &str {
        &self.package.id
    }

    fn icon(&self, name: &str) -> &'static str {
        match self.icons.get(name) {
            Some(full) => full,
            None if name.is_empty() => "tree-feature",
            None => intern(name),
        }
    }

    /// Hand what a call asked for to the host through `ctx`.
    fn settle_aftermath(
        &self,
        inner: &mut Inner,
        aftermath: Aftermath,
        ctx: &mut WorkbenchRuntimeContext,
    ) {
        inner.stale = true;
        for request in aftermath.requests {
            match convert::request(request, &self.package.granted) {
                Ok(request) => ctx.request(request),
                Err(e) => ctx.log_warn(format!("{}: {e}", self.descriptor.label)),
            }
        }
    }

    /// A call that may change the document through `ctx`.
    fn write_call<R>(
        &self,
        inner: &mut Inner,
        ctx: &mut WorkbenchRuntimeContext,
        budget: Budget,
        f: impl FnOnce(
            &crate::host::BenchExports,
            &mut wasmtime::Store<crate::host::State>,
        ) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let access = Access::Write(
            (ctx as *mut WorkbenchRuntimeContext<'_>).cast::<WorkbenchRuntimeContext<'static>>(),
        );
        let (value, aftermath) = inner.guest.call(budget, access, f)?;
        self.settle_aftermath(inner, aftermath, ctx);
        Some(value)
    }

    /// A call that reads `document`. Requests made during it are dropped:
    /// drawing and planning ask nothing of the host.
    fn read_call<R>(
        &self,
        inner: &mut Inner,
        document: &Document,
        budget: Budget,
        f: impl FnOnce(
            &crate::host::BenchExports,
            &mut wasmtime::Store<crate::host::State>,
        ) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let (value, aftermath) =
            inner
                .guest
                .call(budget, Access::Read(document as *const Document), f)?;
        if !aftermath.requests.is_empty() {
            tracing::warn!(target: "printcad.bench", package = %self.id(), "requests made while drawing or planning are dropped");
        }
        if aftermath.redraw {
            inner.stale = true;
        }
        Some(value)
    }

    /// The frame, asked again when the document, the selection or the
    /// bench's own state moved since it was drawn.
    fn refresh<'a>(&self, inner: &'a mut Inner, ctx: &WorkbenchRuntimeContext) -> &'a Frame {
        let key = hash_of((
            ctx.document.mutation_seq(),
            ctx.selected_body_id,
            ctx.selected_face.map(|f| f.point.map(f32::to_bits)),
            ctx.selected_edges
                .iter()
                .map(|e| e.point.map(f32::to_bits))
                .collect::<Vec<_>>(),
            ctx.active_document_object,
        ));
        if inner.stale || inner.frame_key != Some(key) {
            inner.stale = false;
            inner.frame_key = Some(key);
            let id = self.id().to_string();
            let pointer = serde_json::to_string(&convert::pointer(ctx, None)).unwrap_or_default();
            if let Some(json) = self.read_call(inner, ctx.document, Budget::Frame, |b, s| {
                b.call_frame(s, &pointer)
            }) && let Some(frame) = parse::<Frame>(&id, "the frame", &json)
            {
                inner.frame = frame;
            }
            // A redraw asked for during the frame itself waits for the next
            // event rather than asking every frame.
            inner.stale = false;
        }
        &inner.frame
    }

    fn deliver(
        &self,
        inner: &mut Inner,
        ctx: &mut WorkbenchRuntimeContext,
        input: bench_api::Input,
    ) -> bool {
        let Ok(json) = serde_json::to_string(&input) else {
            return false;
        };
        self.write_call(inner, ctx, Budget::Frame, |b, s| b.call_input(s, &json))
            .unwrap_or(false)
    }

    fn deliver_finished_jobs(&self, inner: &mut Inner, ctx: &mut WorkbenchRuntimeContext) {
        for (job, result) in inner.guest.jobs.take_finished() {
            let pointer = convert::pointer(ctx, None);
            let input = bench_api::Input {
                event: bench_api::Event::JobFinished { job, result },
                tool: None,
                pointer,
            };
            let Ok(json) = serde_json::to_string(&input) else {
                continue;
            };
            let _ = self.write_call(inner, ctx, Budget::Long, |b, s| b.call_input(s, &json));
        }
    }

    /// `panel` with every job's progress filled in.
    fn with_progress(inner: &Inner, panel: &[Widget]) -> Vec<Widget> {
        panel
            .iter()
            .map(|w| match w {
                Widget::Progress {
                    label,
                    job: Some(job),
                    fraction,
                } => Widget::Progress {
                    label: label.clone(),
                    fraction: match inner.guest.jobs.progress(*job) {
                        Some((done, total)) if total > 0 => Some(done as f32 / total as f32),
                        Some(_) => None,
                        None => *fraction,
                    },
                    job: Some(*job),
                },
                Widget::Group {
                    title,
                    open,
                    children,
                } => Widget::Group {
                    title: title.clone(),
                    open: *open,
                    children: Self::with_progress(inner, children),
                },
                other => other.clone(),
            })
            .collect()
    }

    fn project(ctx: &WorkbenchRuntimeContext, p: [f32; 3]) -> Option<[f32; 2]> {
        ctx.world_to_viewport(p).map(|(x, y)| [x, y])
    }
}

impl Workbench for WasmWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        self.descriptor.clone()
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        let prefix = format!("{}.", self.id());
        let own = |id: &str, what: &str| {
            let ok = id.starts_with(&prefix);
            if !ok {
                tracing::warn!(target: "printcad.bench", package = %self.id(), "{what} `{id}` is left out: its id must start with `{prefix}`");
            }
            ok
        };
        let chords = |keys: &[String]| -> Vec<Chord> {
            keys.iter()
                .filter_map(|k| {
                    let chord = Chord::parse(k);
                    if chord.is_none() {
                        tracing::warn!(target: "printcad.bench", package = %self.id(), "`{k}` is not a key");
                    }
                    chord
                })
                .collect()
        };
        for tool in &self.registration.tools {
            if !own(&tool.id, "tool") {
                continue;
            }
            let mut descriptor = match tool.behavior {
                bench_api::ToolBehavior::Radio => match &tool.group {
                    Some(group) => ToolDescriptor::new_radio_group(
                        &tool.id,
                        &tool.label,
                        tool.category.clone(),
                        group,
                    ),
                    None => ToolDescriptor::new(&tool.id, &tool.label, tool.category.clone()),
                },
                bench_api::ToolBehavior::Check => {
                    ToolDescriptor::new_check(&tool.id, &tool.label, tool.category.clone())
                }
                bench_api::ToolBehavior::Action => {
                    ToolDescriptor::new_action(&tool.id, &tool.label, tool.category.clone())
                }
            };
            debug_assert!(matches!(
                descriptor.behavior,
                ToolBehavior::Radio | ToolBehavior::Check | ToolBehavior::Action
            ));
            descriptor = descriptor
                .icon(self.icon(&tool.icon))
                .row(tool.row.clamp(1, 2));
            descriptor.shortcuts = chords(&tool.shortcuts);
            context.register_tool(descriptor);
        }
        for action in &self.registration.actions {
            if !own(&action.id, "action") {
                continue;
            }
            let mut descriptor = ActionDescriptor::new(&action.id, &action.label);
            descriptor.shortcuts = chords(&action.shortcuts);
            context.register_action(descriptor);
        }
        for command in &self.registration.commands {
            if !own(&command.id, "command") {
                continue;
            }
            let mut spec = CommandSpec::new(&command.id, &command.summary);
            if !command.returns.is_empty() {
                spec.returns = command.returns.clone();
            }
            spec.read_only = command.read_only;
            spec.params = command
                .params
                .iter()
                .map(|p| core_document::command::ParamSpec {
                    name: p.name.clone(),
                    kind: match p.kind {
                        bench_api::ParamKind::Number => core_document::command::ParamKind::Number,
                        bench_api::ParamKind::Integer => core_document::command::ParamKind::Integer,
                        bench_api::ParamKind::Bool => core_document::command::ParamKind::Bool,
                        bench_api::ParamKind::String => core_document::command::ParamKind::String,
                        bench_api::ParamKind::Id => core_document::command::ParamKind::Id,
                        bench_api::ParamKind::List => core_document::command::ParamKind::List,
                        bench_api::ParamKind::Any => core_document::command::ParamKind::Any,
                    },
                    required: p.required,
                    doc: p.doc.clone(),
                })
                .collect();
            context.register_command(spec);
        }
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        let key = hash_of((
            node.workbench_id.as_str(),
            core_document::feature::node_revision(node),
            &node.made_by,
        ));
        let mut inner = self.inner();
        if let Some(info) = inner.info.get(&key) {
            return info.clone();
        }
        let json = serde_json::to_string(&convert::bare_node(node, &node.data)).unwrap_or_default();
        let answer = inner
            .guest
            .call(Budget::Frame, Access::None, |b, s| {
                b.call_feature_info(s, &json)
            })
            .and_then(|(json, _)| {
                parse::<bench_api::FeatureInfo>(self.id(), "feature info", &json)
            });
        let info = match answer {
            Some(info) => FeatureInfo {
                icon: self.icon(&info.icon),
                kind_label: info.kind_label,
                family_label: if info.family_label.is_empty() {
                    self.descriptor.label.clone()
                } else {
                    info.family_label
                },
                builds_solid: info.builds_solid,
            },
            None => FeatureInfo::fallback(node),
        };
        if inner.info.len() > CACHE_LIMIT {
            inner.info.clear();
        }
        inner.info.insert(key, info.clone());
        info
    }

    fn locks_view_to_plane(&self) -> bool {
        self.inner().frame.locks_view
    }

    fn takes_numeric_input(&self) -> bool {
        self.inner().frame.numeric_input
    }

    fn busy(&self) -> bool {
        // A finished job waits for the bench to be active to be told.
        let inner = self.inner();
        inner.guest.jobs.running() || (inner.active && inner.guest.jobs.has_finished())
    }

    fn menu_items(&self, scope: &MenuScope, document: &Document) -> Vec<MenuItem> {
        let Ok(json) = serde_json::to_string(&convert::menu_scope(scope)) else {
            return Vec::new();
        };
        let mut inner = self.inner();
        let answer = self.read_call(&mut inner, document, Budget::Frame, |b, s| {
            b.call_menu_items(s, &json)
        });
        answer
            .and_then(|json| parse::<Vec<bench_api::MenuItem>>(self.id(), "menu items", &json))
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                let mut out = MenuItem::new(item.id, item.label).enabled(item.enabled);
                if let Some(icon) = item.icon {
                    out = out.icon(self.icon(&icon));
                }
                if let Some(hint) = item.hint {
                    out = out.hint(hint);
                }
                if item.separator_before {
                    out = out.separator_before();
                }
                out
            })
            .collect()
    }

    fn on_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        let Ok(scope) = serde_json::to_string(&convert::menu_scope(scope)) else {
            return false;
        };
        let mut inner = self.inner();
        self.write_call(&mut inner, ctx, Budget::Long, |b, s| {
            b.call_menu_command(s, id, &scope)
        })
        .unwrap_or(false)
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let args = serde_json::to_string(args).map_err(|e| CommandError::failed(e.to_string()))?;
        let mut inner = self.inner();
        let answer = self
            .write_call(&mut inner, ctx, Budget::Long, |b, s| {
                b.call_run_command(s, id, &args)
            })
            .ok_or_else(|| CommandError::failed(format!("the workbench {} failed", self.id())))?;
        match answer {
            Ok(json) if json.trim().is_empty() => Ok(serde_json::Value::Null),
            Ok(json) => serde_json::from_str(&json)
                .map_err(|e| CommandError::failed(format!("the answer is not JSON: {e}"))),
            Err(message) => Err(CommandError::failed(message)),
        }
    }

    fn parameters(&self, node: &FeatureNode) -> Vec<Parameter> {
        let key = hash_of((
            node.workbench_id.as_str(),
            core_document::feature::node_revision(node),
        ));
        let mut inner = self.inner();
        if let Some(params) = inner.params.get(&key) {
            return params.clone();
        }
        let json = serde_json::to_string(&convert::bare_node(node, &node.data)).unwrap_or_default();
        let params: Vec<Parameter> = inner
            .guest
            .call(Budget::Frame, Access::None, |b, s| {
                b.call_parameters(s, &json)
            })
            .and_then(|(json, _)| {
                parse::<Vec<bench_api::Parameter>>(self.id(), "parameters", &json)
            })
            .unwrap_or_default()
            .into_iter()
            .map(convert::parameter)
            .collect();
        if inner.params.len() > CACHE_LIMIT {
            inner.params.clear();
        }
        inner.params.insert(key, params.clone());
        params
    }

    fn values_moved(&mut self, ctx: &mut WorkbenchRuntimeContext, moved: &[FeatureId]) {
        let owned: Vec<String> = moved
            .iter()
            .filter(|id| {
                ctx.document
                    .get_feature_meta(**id)
                    .is_some_and(|n| self.package.owns(n.workbench_id.as_str()))
            })
            .map(|id| id.0.to_string())
            .collect();
        if owned.is_empty() {
            return;
        }
        let input = bench_api::Input {
            event: bench_api::Event::ValuesMoved { features: owned },
            tool: None,
            pointer: convert::pointer(ctx, None),
        };
        let mut inner = self.inner();
        self.deliver(&mut inner, ctx, input);
    }

    fn settle(&self, node: &FeatureNode, values: &mut serde_json::Value) {
        let Ok(json) = serde_json::to_string(&convert::bare_node(node, values)) else {
            return;
        };
        let mut inner = self.inner();
        if let Some((json, _)) = inner
            .guest
            .call(Budget::Long, Access::None, |b, s| b.call_settle(s, &json))
            && let Some(settled) = parse::<serde_json::Value>(self.id(), "settled data", &json)
        {
            *values = settled;
        }
    }

    fn property_hints(&self) -> PropertyHints {
        PropertyHints {
            length_keys: self
                .registration
                .length_keys
                .iter()
                .map(|k| intern(k))
                .collect(),
            reference_keys: Vec::new(),
        }
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<RebuildJob> {
        let dirty: Vec<(FeatureId, Option<BodyId>)> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.dirty && self.package.owns(n.workbench_id.as_str()))
            .map(|(id, n)| (*id, n.body))
            .collect();
        if dirty.is_empty() {
            return Vec::new();
        }
        let mut bodies: Vec<BodyId> = Vec::new();
        for (id, body) in &dirty {
            match body {
                Some(body) if !bodies.contains(body) => bodies.push(*body),
                Some(_) => {}
                None => document.clear_feature_dirty(*id),
            }
        }
        let mut request = bench_api::RebuildRequest::default();
        for body in &bodies {
            let tip_seq = document
                .bodies()
                .iter()
                .find(|b| b.id == *body)
                .and_then(|b| b.tip)
                .and_then(|tip| document.get_feature_meta(tip))
                .map(|n| n.seq);
            let features = owned_in_body(document, &self.package, *body)
                .into_iter()
                .filter_map(|id| document.get_feature_meta(id))
                .filter(|n| !n.suppressed && tip_seq.is_none_or(|seq| n.seq <= seq))
                .map(|n| convert::node(document, n))
                .collect();
            request.bodies.push(bench_api::BodyHistory {
                body: body.0.to_string(),
                features,
            });
        }
        // Settled before planning, or a plan that fails comes back every
        // frame: the features planned and the dirty features they read.
        for body in &bodies {
            for id in owned_in_body(document, &self.package, *body) {
                for dep in document.feature_tree().dependencies(id) {
                    if document.get_feature_meta(dep).is_some_and(|n| n.dirty) {
                        document.clear_feature_dirty(dep);
                    }
                }
                document.clear_feature_dirty(id);
            }
        }
        let Ok(json) = serde_json::to_string(&request) else {
            return Vec::new();
        };
        let mut inner = self.inner();
        let answer = self.read_call(&mut inner, document, Budget::Long, |b, s| {
            b.call_rebuild(s, &json)
        });
        let Some(rebuilds) = answer.and_then(|json| {
            parse::<Vec<bench_api::Rebuild>>(self.id(), "the rebuild plans", &json)
        }) else {
            return bodies
                .into_iter()
                .map(|body| RebuildJob {
                    body,
                    plan: Err(BuildError {
                        feature: owned_in_body(document, &self.package, body).last().copied(),
                        message: format!("the workbench {} could not plan the rebuild", self.id()),
                    }),
                })
                .collect();
        };
        rebuilds
            .into_iter()
            .filter_map(|rebuild| {
                let body = crate::host::body_id(&rebuild.body).ok()?;
                if !bodies.contains(&body) {
                    return None;
                }
                let plan = match rebuild.plan {
                    bench_api::Plan::Ops { ops, op_features } => {
                        let features: Result<Vec<FeatureId>, String> = op_features
                            .iter()
                            .map(|f| crate::host::feature_id(f))
                            .collect();
                        match features {
                            Ok(op_features) if op_features.len() == ops.len() => {
                                Ok(BuildPlan { ops, op_features })
                            }
                            Ok(_) => Err(BuildError {
                                feature: None,
                                message: "the plan names a feature for each op, and it does not"
                                    .into(),
                            }),
                            Err(e) => Err(BuildError {
                                feature: None,
                                message: e,
                            }),
                        }
                    }
                    bench_api::Plan::Empty => Ok(BuildPlan {
                        ops: Vec::new(),
                        op_features: Vec::new(),
                    }),
                    bench_api::Plan::Error { feature, message } => Err(BuildError {
                        feature: feature.and_then(|f| crate::host::feature_id(&f).ok()),
                        message,
                    }),
                };
                Some(RebuildJob { body, plan })
            })
            .collect()
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        invalidate_owned(document, &self.package, body);
    }

    fn invalidate_all(&self, document: &mut Document) {
        let ids: Vec<FeatureId> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| self.package.owns(n.workbench_id.as_str()))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            document.mark_feature_dirty(id);
        }
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let mut inner = self.inner();
        inner.active = true;
        let input = bench_api::Input {
            event: bench_api::Event::Activated,
            tool: None,
            pointer: convert::pointer(ctx, None),
        };
        self.deliver(&mut inner, ctx, input);
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let mut inner = self.inner();
        inner.active = false;
        let input = bench_api::Input {
            event: bench_api::Event::Deactivated,
            tool: None,
            pointer: convert::pointer(ctx, None),
        };
        self.deliver(&mut inner, ctx, input);
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        let mut inner = self.inner();
        self.deliver_finished_jobs(&mut inner, ctx);
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        let (event, position) = convert::event(event);
        let input = bench_api::Input {
            event,
            tool: active_tool.map(str::to_string),
            pointer: convert::pointer(ctx, position),
        };
        let mut inner = self.inner();
        if self.deliver(&mut inner, ctx, input) {
            InputResult::consumed()
        } else {
            InputResult::redraw_only()
        }
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        let mut inner = self.inner();
        let task = self.refresh(&mut inner, ctx).task.clone()?;
        Some(TaskInfo {
            title: task.title,
            icon: self.icon(&task.icon),
            confirmable: task.confirmable,
            stepwise: false,
        })
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        use core_document::TaskOutcome;
        let mut inner = self.inner();
        let panel = {
            let frame = self.refresh(&mut inner, ctx);
            frame.panel.clone()
        };
        let panel = Self::with_progress(&inner, &panel);
        let out = core_document::panel::show(
            ui,
            egui::Id::new(("bench_panel", self.id())),
            &panel,
            Some(&*ctx.document),
        );
        core_document::panel::apply_formulas(ctx.document, out.formulas);
        for event in out.events {
            let Ok(json) = serde_json::to_string(&event) else {
                continue;
            };
            let _ = self.write_call(&mut inner, ctx, Budget::Long, |b, s| {
                b.call_panel_event(s, "task", &json)
            });
        }
        if request.accept || request.cancel {
            let title = inner
                .frame
                .task
                .as_ref()
                .map(|t| t.title.clone())
                .unwrap_or_default();
            let label = self.write_call(&mut inner, ctx, Budget::Long, |b, s| {
                b.call_task_close(s, request.accept)
            });
            return if request.accept {
                TaskOutcome::Accepted {
                    label: label.flatten().unwrap_or(title),
                }
            } else {
                TaskOutcome::Cancelled
            };
        }
        TaskOutcome::Open
    }

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui, _filter: &str) -> bool {
        let mut inner = self.inner();
        if inner.settings_panel.is_none() {
            let panel = inner
                .guest
                .call(Budget::Long, Access::None, |b, s| b.call_settings_panel(s))
                .and_then(|(json, _)| parse::<Vec<Widget>>(self.id(), "the settings page", &json))
                .unwrap_or_default();
            inner.settings_panel = Some(panel);
        }
        let panel = inner.settings_panel.clone().unwrap_or_default();
        let out = core_document::panel::show(
            ui,
            egui::Id::new(("bench_settings", self.id())),
            &panel,
            None,
        );
        let changed = !out.events.is_empty();
        for event in out.events {
            let Ok(json) = serde_json::to_string(&event) else {
                continue;
            };
            let _ = inner.guest.call(Budget::Long, Access::None, |b, s| {
                b.call_panel_event(s, "settings", &json)
            });
        }
        if changed {
            inner.settings_panel = None;
        }
        changed
    }

    fn viewport_hud(&self, ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let mut inner = self.inner();
        let hud = self.refresh(&mut inner, ctx).hud.clone();
        let empty = hud.tool.is_none()
            && hud.badge.is_none()
            && hud.legend.is_empty()
            && hud.footer.is_empty();
        if empty {
            return None;
        }
        Some(ViewportHud {
            tool: hud.tool.map(|t| ToolHint {
                icon: self.icon(&t.icon),
                name: t.name,
                prompt: t.prompt,
                keys: t.keys.into_iter().map(|(k, m)| (k, intern(&m))).collect(),
            }),
            badge: hud.badge,
            legend: hud
                .legend
                .into_iter()
                .map(|(c, l)| (c, intern(&l)))
                .collect(),
            footer: hud.footer,
            ovp: None,
        })
    }

    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<StatusItems> {
        let mut inner = self.inner();
        let status = self.refresh(&mut inner, ctx).status.clone();
        if status == bench_api::Status::default() {
            return None;
        }
        Some(StatusItems {
            state: status.state,
            selection: status.selection,
            coords: None,
            mode: status.mode,
        })
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        let inner = self.inner();
        inner
            .frame
            .editing
            .as_deref()
            .and_then(|id| crate::host::feature_id(id).ok())
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        self.inner().frame.toggled.iter().any(|t| t == tool_id)
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let mut inner = self.inner();
        !self
            .refresh(&mut inner, ctx)
            .disabled
            .iter()
            .any(|t| t == tool_id)
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let input = bench_api::Input {
            event: bench_api::Event::Action {
                id: "finish-editing".into(),
            },
            tool: None,
            pointer: convert::pointer(ctx, None),
        };
        let mut inner = self.inner();
        self.deliver(&mut inner, ctx, input);
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        let mut inner = self.inner();
        let (json, _) = inner
            .guest
            .call(Budget::Long, Access::None, |b, s| b.call_settings(s))?;
        let json = json?;
        inner.guest.settings = Some(json.clone());
        serde_json::from_str(&json).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        let json = value.to_string();
        let mut inner = self.inner();
        inner.guest.settings = Some(json.clone());
        inner.settings_panel = None;
        let _ = inner.guest.call(Budget::Long, Access::None, |b, s| {
            b.call_apply_settings(s, &json)
        });
    }

    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        let mut inner = self.inner();
        inner.stale = true;
        let (state, _) = inner
            .guest
            .call(Budget::Long, Access::None, |b, s| b.call_suspend(s))?;
        state.map(|bytes| Box::new(bytes) as Box<dyn std::any::Any + Send>)
    }

    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        let bytes = state.and_then(|s| s.downcast::<Vec<u8>>().ok()).map(|b| *b);
        let mut inner = self.inner();
        inner.stale = true;
        let _ = inner.guest.call(Budget::Long, Access::None, |b, s| {
            b.call_resume(s, bytes.as_deref())
        });
    }

    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        let text = id.0.to_string();
        let body = ctx.document.get_feature_meta(id).and_then(|n| n.body);
        let mut inner = self.inner();
        let handled = self
            .write_call(&mut inner, ctx, Budget::Long, |b, s| {
                b.call_delete_feature(s, &text)
            })
            .unwrap_or(false);
        if handled {
            return ctx.document.get_feature_meta(id).is_none();
        }
        let removed = ctx.document.remove_feature(id).is_ok();
        if removed && let Some(body) = body {
            invalidate_owned(ctx.document, &self.package, body);
        }
        removed
    }

    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<OverlayMesh> {
        let mut inner = self.inner();
        self.refresh(&mut inner, ctx)
            .meshes
            .iter()
            .map(|m| OverlayMesh {
                mesh: convert::tri_mesh(m.positions.clone(), m.indices.clone()),
                color: m.color,
                wireframe: m.wireframe,
                opacity: m.opacity.clamp(0.0, 1.0),
                on_top: m.on_top,
            })
            .collect()
    }

    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceOverlay> {
        let mut inner = self.inner();
        let frame = self.refresh(&mut inner, ctx);
        let mut out = Vec::new();
        for line in &frame.lines {
            let mut points = line.points.clone();
            if line.closed
                && let Some(first) = points.first().copied()
            {
                points.push(first);
            }
            for pair in points.windows(2) {
                let (Some(a), Some(b)) = (Self::project(ctx, pair[0]), Self::project(ctx, pair[1]))
                else {
                    continue;
                };
                let mut segment = ScreenSpaceOverlay::new(a, b, line.color, line.width);
                if line.dashed {
                    segment = segment.dashed(6.0, 4.0);
                }
                out.push(segment);
            }
        }
        out
    }

    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceMark> {
        let mut inner = self.inner();
        let marks = self.refresh(&mut inner, ctx).marks.clone();
        marks
            .into_iter()
            .filter_map(|mark| {
                let pos = Self::project(ctx, mark.at)?;
                Some(match mark.kind {
                    bench_api::MarkKind::Dot { radius } => {
                        ScreenSpaceMark::dot(pos, radius, mark.color)
                    }
                    bench_api::MarkKind::Cross { size } => {
                        ScreenSpaceMark::crosshair(pos, size, mark.color)
                    }
                    bench_api::MarkKind::Icon { name, size } => ScreenSpaceMark {
                        pos,
                        color: mark.color,
                        alpha: 1.0,
                        kind: MarkKind::Icon {
                            name: self.icon(&name),
                            size,
                        },
                    },
                })
            })
            .collect()
    }

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        let mut inner = self.inner();
        let labels = self.refresh(&mut inner, ctx).labels.clone();
        labels
            .into_iter()
            .filter_map(|label| {
                let pos = Self::project(ctx, label.at)?;
                let mut out = ScreenSpaceLabel::new(pos, label.text, label.color, label.size);
                if label.pill {
                    out = out.pill();
                }
                if label.mono {
                    out = out.mono();
                }
                Some(out)
            })
            .collect()
    }
}

impl Drop for WasmWorkbench {
    fn drop(&mut self) {
        self.inner().guest.jobs.cancel_all();
    }
}
