//! The host side of the interface: what a guest reaches through `host`,
//! and what it may reach during each kind of call. The runtime holding
//! the instance hands each import to [`Reach`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bench_api::{Capabilities, Request, calls};
use core_document::{BodyPlacement, Document, FeatureId, WorkbenchId, WorkbenchRuntimeContext};
use serde_json::{Value, json};

use crate::jobs::JobBoard;

/// Where a feature's closed loops come from: a function of the bench that
/// draws them, answering in the feature's body frame.
pub type ProfileSource = fn(&Document, FeatureId) -> Option<kernel_api::Profile>;

static PROFILES: std::sync::OnceLock<ProfileSource> = std::sync::OnceLock::new();

/// Name where packages read features' closed loops (`host.profile`); the
/// first one named holds. Without one, no feature has a profile.
pub fn set_profile_source(source: ProfileSource) {
    let _ = PROFILES.set(source);
}

/// A feature's closed loops placed where its body sits.
fn placed_profile(document: &Document, id: FeatureId) -> Option<kernel_api::Profile> {
    let mut profile = PROFILES.get()?(document, id)?;
    let placement = document
        .get_feature_meta(id)?
        .body
        .map(|body| document.body_placement(body))
        .unwrap_or_default();
    if !placement.is_identity() {
        let plane = &mut profile.plane;
        let point = |p: [f64; 3]| placement.point(p.map(|c| c as f32)).map(f64::from);
        let turn = |d: [f64; 3]| placement.direction(d.map(|c| c as f32)).map(f64::from);
        plane.origin = point(plane.origin);
        plane.x_axis = turn(plane.x_axis);
        plane.y_axis = turn(plane.y_axis);
        plane.normal = turn(plane.normal);
    }
    Some(profile)
}

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

/// The host's side of one instance: what the call in progress may reach
/// and what it left for the host.
pub(crate) struct Reach {
    pub package: Arc<PackageInfo>,
    pub access: Access,
    pub requests: Vec<Request>,
    pub redraw: bool,
    pub jobs: Arc<JobBoard>,
    /// Set in an instance that runs a job.
    pub job: Option<JobLine>,
}

// SAFETY: `access` points at a document or context only for the length of
// one synchronous call made on the thread that holds the borrow (the
// guest's `call` sets it and resets it before returning); at any other
// time it is `Access::None`. Everything else in it is `Send`.
unsafe impl Send for Reach {}

impl Reach {
    pub(crate) fn new(package: Arc<PackageInfo>, jobs: Arc<JobBoard>) -> Self {
        Self {
            package,
            access: Access::None,
            requests: Vec::new(),
            redraw: false,
            jobs,
            job: None,
        }
    }

    fn document(&self) -> Option<&Document> {
        // SAFETY: see `unsafe impl Send for Reach`: a pointer here is live
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
        if [
            calls::GEOM_HOLES,
            calls::GEOM_FACE_EDGES,
            calls::GEOM_SECTION,
            calls::GEOM_FACE_LOOPS,
        ]
        .contains(&command)
        {
            return crate::geometry::call(ctx, command, &args);
        }
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
            calls::REMOVE_BODY => {
                let text = str_arg(&args, "id")?;
                let body = body_id(text)?;
                if !document.bodies().iter().any(|b| b.id == body) {
                    return Err(format!("no body {text}"));
                }
                let foreign = document
                    .feature_tree()
                    .all_nodes()
                    .find(|(_, n)| n.body == Some(body) && !package.owns(n.workbench_id.as_str()))
                    .map(|(_, n)| n.workbench_id.as_str().to_string());
                if let Some(kind) = foreign {
                    return Err(format!(
                        "body {text} holds a {kind}, which the package does not own"
                    ));
                }
                if document.body_solid_is_imported(body) {
                    return Err(format!(
                        "body {text} takes its shape from an import, a link or a copy, \
                         which the package does not own"
                    ));
                }
                document.remove_body(body);
                Ok(Value::Null)
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
                    calls::REMOVE_BODY,
                    calls::SET_PLACEMENT,
                    calls::JOB_START,
                    calls::JOB_CANCEL,
                    calls::GEOM_HOLES,
                    calls::GEOM_FACE_EDGES,
                    calls::GEOM_SECTION,
                    calls::GEOM_FACE_LOOPS,
                ]
                .join(", ")
            )),
        }
    }
}

/// The guest's imports, `host` in the WIT world.
impl Reach {
    pub(crate) fn log(&mut self, level: String, message: String) {
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

    pub(crate) fn feature(&mut self, id: String) -> Option<String> {
        let document = self.document()?;
        let id = feature_id(&id).ok()?;
        let node = document.get_feature_meta(id)?;
        serde_json::to_string(&crate::convert::node(document, node)).ok()
    }

    pub(crate) fn features(&mut self) -> String {
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

    pub(crate) fn bodies(&mut self) -> String {
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

    /// Positions as x, y, z triples, and the triangles' indices.
    pub(crate) fn body_mesh(&mut self, body: String) -> Option<(Vec<f32>, Vec<u32>)> {
        let document = self.document()?;
        let geometry = document.imported_geometry(body_id(&body).ok()?)?;
        Some((
            geometry.mesh.positions.iter().flatten().copied().collect(),
            geometry.mesh.indices.clone(),
        ))
    }

    pub(crate) fn body_shape(&mut self, body: String) -> Option<Vec<u8>> {
        let document = self.document()?;
        document
            .imported_brep_blob(body_id(&body).ok()?)
            .map(<[u8]>::to_vec)
    }

    pub(crate) fn profile(&mut self, feature: String) -> Option<String> {
        let document = self.document()?;
        let profile = placed_profile(document, feature_id(&feature).ok()?)?;
        serde_json::to_string(&profile).ok()
    }

    pub(crate) fn call(&mut self, command: String, args: String) -> Result<String, String> {
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

    pub(crate) fn request(&mut self, request: String) {
        match serde_json::from_str::<Request>(&request) {
            Ok(request) => self.requests.push(request),
            Err(e) => {
                let id = self.package.id.clone();
                tracing::warn!(target: "printcad.bench", package = %id, "a request that is not one: {e}");
            }
        }
    }

    pub(crate) fn redraw(&mut self) {
        self.redraw = true;
    }

    pub(crate) fn progress(&mut self, done: u64, total: u64) {
        if let Some(job) = &self.job {
            job.done.store(done, Ordering::Relaxed);
            job.total.store(total, Ordering::Relaxed);
        }
    }

    pub(crate) fn cancelled(&mut self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| j.cancelled.load(Ordering::Relaxed))
    }

    pub(crate) fn helper(&mut self, name: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        let Some(job) = &self.job else {
            return Err("helpers run inside a job".into());
        };
        if !self.package.granted.helper {
            return Err("the package has not been allowed to run helpers".into());
        }
        crate::jobs::run_helper(&self.package.helpers, &name, &input, &job.cancelled)
    }
}

pub(crate) fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
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
