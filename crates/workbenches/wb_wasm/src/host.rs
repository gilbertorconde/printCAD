//! The host side of the interface: what a guest reaches through `host`,
//! and what it may reach during each kind of call.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bench_api::{Capabilities, Request, calls};
use core_document::{BodyPlacement, Document, FeatureId, WorkbenchId, WorkbenchRuntimeContext};
use serde_json::{Value, json};
use wasmtime::StoreLimits;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::jobs::JobBoard;

wasmtime::component::bindgen!({
    world: "workbench",
    path: "../../bench_api/wit",
});

pub(crate) use exports::printcad::workbench::bench::Guest as BenchExports;
pub(crate) use printcad::workbench::host::Mesh;

/// What the package is, for every call its instances make.
#[derive(Debug)]
pub(crate) struct PackageInfo {
    pub id: String,
    pub version: String,
    pub kinds: Vec<String>,
    pub granted: Capabilities,
    /// `helpers/<os>-<arch>/` in the package.
    pub helpers: PathBuf,
    /// Where the package is published (`owner/repo`), when it was
    /// installed from there; written on the features it makes.
    pub source: Option<String>,
}

impl PackageInfo {
    pub(crate) fn made_by(&self) -> String {
        format!("{} {}", self.id, self.version)
    }

    /// What features this package writes record of it.
    pub(crate) fn origin(&self) -> core_document::FeatureOrigin {
        core_document::FeatureOrigin::new(self.made_by(), self.source.clone())
    }

    pub(crate) fn owns(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k == kind)
    }
}

/// What the call in progress may reach.
#[derive(Clone, Copy, Default)]
pub(crate) enum Access {
    /// Nothing of the document: presentation of one node, settings.
    #[default]
    None,
    /// Read the document: drawing, planning a rebuild, menus.
    Read(*const Document),
    /// Read and change it: events, panel changes, commands.
    Write(*mut WorkbenchRuntimeContext<'static>),
}

/// A job's own line back to the host.
pub(crate) struct JobLine {
    pub cancelled: Arc<AtomicBool>,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

/// The store's data: WASI, limits, and the host's side of one call.
pub(crate) struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    pub limits: StoreLimits,
    pub package: Arc<PackageInfo>,
    pub access: Access,
    pub requests: Vec<Request>,
    pub redraw: bool,
    pub jobs: Arc<JobBoard>,
    /// Set in an instance that runs a job.
    pub job: Option<JobLine>,
}

// SAFETY: `access` points at a document or context only for the length of
// one synchronous call made on the thread that holds the borrow
// (`Guest::call` sets it and resets it before returning); at any other time
// it is `Access::None`. Everything else in the state is `Send`.
unsafe impl Send for State {}

impl State {
    pub(crate) fn new(
        wasi: WasiCtx,
        limits: StoreLimits,
        package: Arc<PackageInfo>,
        jobs: Arc<JobBoard>,
    ) -> Self {
        Self {
            wasi,
            table: ResourceTable::new(),
            limits,
            package,
            access: Access::None,
            requests: Vec::new(),
            redraw: false,
            jobs,
            job: None,
        }
    }

    fn document(&self) -> Option<&Document> {
        // SAFETY: see `unsafe impl Send for State`: a pointer here is live
        // for the call in progress.
        unsafe {
            match self.access {
                Access::None => None,
                Access::Read(doc) => Some(&*doc),
                Access::Write(ctx) => Some(&*(*ctx).document),
            }
        }
    }

    fn context(&mut self) -> Option<&mut WorkbenchRuntimeContext<'static>> {
        // SAFETY: as in `document`.
        unsafe {
            match self.access {
                Access::Write(ctx) => Some(&mut *ctx),
                _ => None,
            }
        }
    }

    /// Run one document command for the guest.
    fn run_call(&mut self, command: &str, args: Value) -> Result<Value, String> {
        if command == calls::JOB_START {
            if self.job.is_some() {
                return Err("a job cannot start another job".into());
            }
            let entry = str_arg(&args, "entry")?.to_string();
            let input = args
                .get("input")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let job = self.jobs.start(entry, input)?;
            return Ok(json!(job));
        }
        if command == calls::JOB_CANCEL {
            let job = args
                .get("job")
                .and_then(Value::as_u64)
                .ok_or("argument `job` is required")?;
            self.jobs.cancel(job);
            return Ok(Value::Null);
        }
        let package = self.package.clone();
        let Some(ctx) = self.context() else {
            return Err(format!(
                "`{command}` changes the document, which this call may only read"
            ));
        };
        let document = &mut *ctx.document;
        match command {
            calls::ADD_FEATURE => {
                let kind = str_arg(&args, "kind")?;
                if !package.owns(kind) {
                    return Err(format!("the package does not own feature kind `{kind}`"));
                }
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(kind)
                    .to_string();
                let body = match args.get("body").and_then(Value::as_str) {
                    Some(b) => {
                        let body = body_id(b)?;
                        if !document.bodies().iter().any(|x| x.id == body) {
                            return Err(format!("no body {b}"));
                        }
                        Some(body)
                    }
                    None => None,
                };
                let deps = match args.get("deps").and_then(Value::as_array) {
                    Some(list) => list
                        .iter()
                        .map(|v| feature_id(v.as_str().unwrap_or("")))
                        .collect::<Result<Vec<_>, _>>()?,
                    None => Vec::new(),
                };
                let data = args.get("data").cloned().unwrap_or(Value::Null);
                let id = document.add_feature_of_kind(
                    WorkbenchId::new(kind),
                    name,
                    body,
                    deps,
                    data,
                    package.origin(),
                );
                document.mark_feature_dirty(id);
                Ok(json!(id.0.to_string()))
            }
            calls::SET_FEATURE_DATA => {
                let id = owned(document, &package, str_arg(&args, "id")?)?;
                let data = args
                    .get("data")
                    .cloned()
                    .ok_or("argument `data` is required")?;
                document
                    .update_feature_data(id, data)
                    .map_err(|e| e.to_string())?;
                // The data is in this version's form now.
                document
                    .set_feature_origin(id, package.origin())
                    .map_err(|e| e.to_string())?;
                document.mark_feature_dirty(id);
                Ok(Value::Null)
            }
            calls::REMOVE_FEATURE => {
                let id = owned(document, &package, str_arg(&args, "id")?)?;
                let body = document.get_feature_meta(id).and_then(|n| n.body);
                document.remove_feature(id).map_err(|e| e.to_string())?;
                if let Some(body) = body {
                    crate::bench::invalidate_owned(document, &package, body);
                }
                Ok(Value::Null)
            }
            calls::RENAME_FEATURE => {
                let id = owned(document, &package, str_arg(&args, "id")?)?;
                document.rename_feature(id, str_arg(&args, "name")?);
                Ok(Value::Null)
            }
            calls::SET_FEATURE_VISIBLE => {
                let id = owned(document, &package, str_arg(&args, "id")?)?;
                let visible = args
                    .get("visible")
                    .and_then(Value::as_bool)
                    .ok_or("argument `visible` is required")?;
                document.set_feature_visible(id, visible);
                Ok(Value::Null)
            }
            calls::CREATE_BODY => {
                let name = args.get("name").and_then(Value::as_str).map(str::to_string);
                Ok(json!(document.create_body(name).0.to_string()))
            }
            calls::SET_PLACEMENT => {
                let body = body_id(str_arg(&args, "body")?)?;
                let rows: Vec<f64> = args
                    .get("matrix")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                let placement = placement_from_rows(&rows)
                    .ok_or("argument `matrix` must be 16 numbers, a rigid motion row by row")?;
                document.set_body_placement(body, placement);
                Ok(Value::Null)
            }
            other => Err(format!(
                "no command `{other}` for a workbench package (it has {})",
                [
                    calls::ADD_FEATURE,
                    calls::SET_FEATURE_DATA,
                    calls::REMOVE_FEATURE,
                    calls::RENAME_FEATURE,
                    calls::SET_FEATURE_VISIBLE,
                    calls::CREATE_BODY,
                    calls::SET_PLACEMENT,
                    calls::JOB_START,
                    calls::JOB_CANCEL,
                ]
                .join(", ")
            )),
        }
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl printcad::workbench::host::Host for State {
    fn log(&mut self, level: String, message: String) {
        let id = self.package.id.clone();
        match self.context() {
            Some(ctx) => match level.as_str() {
                "error" => ctx.log_error(message),
                "warn" => ctx.log_warn(message),
                _ => ctx.log_info(message),
            },
            None => match level.as_str() {
                "error" => tracing::error!(target: "printcad.bench", package = %id, "{message}"),
                "warn" => tracing::warn!(target: "printcad.bench", package = %id, "{message}"),
                _ => tracing::info!(target: "printcad.bench", package = %id, "{message}"),
            },
        }
    }

    fn feature(&mut self, id: String) -> Option<String> {
        let document = self.document()?;
        let id = feature_id(&id).ok()?;
        let node = document.get_feature_meta(id)?;
        serde_json::to_string(&crate::convert::node(document, node)).ok()
    }

    fn features(&mut self) -> String {
        let Some(document) = self.document() else {
            return "[]".into();
        };
        let mut nodes: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .map(|(_, node)| crate::convert::node(document, node))
            .collect();
        nodes.sort_by_key(|n| n.seq);
        serde_json::to_string(&nodes).unwrap_or_else(|_| "[]".into())
    }

    fn bodies(&mut self) -> String {
        let Some(document) = self.document() else {
            return "[]".into();
        };
        let bodies: Vec<bench_api::Body> = document
            .bodies()
            .iter()
            .map(|b| bench_api::Body {
                id: b.id.0.to_string(),
                name: b.name.clone(),
                hidden: b.hidden,
                placement: {
                    let rows = document.body_placement(b.id).rows();
                    let mut flat = [0.0; 16];
                    for (r, row) in rows.iter().enumerate() {
                        flat[r * 4..r * 4 + 4].copy_from_slice(row);
                    }
                    flat
                },
                bounds: document.imported_geometry(b.id).and_then(|g| g.bounds_mm),
            })
            .collect();
        serde_json::to_string(&bodies).unwrap_or_else(|_| "[]".into())
    }

    fn body_mesh(&mut self, body: String) -> Option<Mesh> {
        let document = self.document()?;
        let geometry = document.imported_geometry(body_id(&body).ok()?)?;
        Some(Mesh {
            positions: geometry.mesh.positions.iter().flatten().copied().collect(),
            indices: geometry.mesh.indices.clone(),
        })
    }

    fn body_shape(&mut self, body: String) -> Option<Vec<u8>> {
        let document = self.document()?;
        document
            .imported_brep_blob(body_id(&body).ok()?)
            .map(<[u8]>::to_vec)
    }

    fn call(&mut self, command: String, args: String) -> Result<String, String> {
        let args: Value = if args.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(&args).map_err(|e| format!("arguments are not JSON: {e}"))?
        };
        let answer = self.run_call(&command, args)?;
        if !matches!(self.access, Access::None | Access::Read(_)) {
            self.redraw = true;
        }
        Ok(answer.to_string())
    }

    fn request(&mut self, request: String) {
        match serde_json::from_str::<Request>(&request) {
            Ok(request) => self.requests.push(request),
            Err(e) => {
                let id = self.package.id.clone();
                tracing::warn!(target: "printcad.bench", package = %id, "a request that is not one: {e}");
            }
        }
    }

    fn redraw(&mut self) {
        self.redraw = true;
    }

    fn progress(&mut self, done: u64, total: u64) {
        if let Some(job) = &self.job {
            job.done.store(done, Ordering::Relaxed);
            job.total.store(total, Ordering::Relaxed);
        }
    }

    fn cancelled(&mut self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| j.cancelled.load(Ordering::Relaxed))
    }

    fn helper(&mut self, name: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        let Some(job) = &self.job else {
            return Err("helpers run inside a job".into());
        };
        if !self.package.granted.helper {
            return Err("the package has not been allowed to run helpers".into());
        }
        crate::jobs::run_helper(&self.package.helpers, &name, &input, &job.cancelled)
    }
}

fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument `{name}` is required"))
}

pub(crate) fn feature_id(text: &str) -> Result<FeatureId, String> {
    uuid::Uuid::parse_str(text)
        .map(FeatureId)
        .map_err(|_| format!("`{text}` is not a feature id"))
}

pub(crate) fn body_id(text: &str) -> Result<core_document::BodyId, String> {
    uuid::Uuid::parse_str(text)
        .map(core_document::BodyId)
        .map_err(|_| format!("`{text}` is not a body id"))
}

/// An owned feature's id.
fn owned(document: &Document, package: &PackageInfo, text: &str) -> Result<FeatureId, String> {
    let id = feature_id(text)?;
    let node = document
        .get_feature_meta(id)
        .ok_or_else(|| format!("no feature {text}"))?;
    if !package.owns(node.workbench_id.as_str()) {
        return Err(format!(
            "feature {text} is a {}, which the package does not own",
            node.workbench_id.as_str()
        ));
    }
    Ok(id)
}

/// A rigid motion from a row-major 4x4 matrix.
pub(crate) fn placement_from_rows(rows: &[f64]) -> Option<BodyPlacement> {
    if rows.len() != 16 || rows.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let m = glam::DMat4::from_cols_array(&rows.try_into().ok()?).transpose();
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    if (scale - glam::DVec3::ONE).abs().max_element() > 1e-6 {
        return None;
    }
    Some(BodyPlacement::new(
        rotation.as_quat(),
        translation.as_vec3(),
    ))
}
