//! Scripts and the console: the Lua engine run against every command the
//! application and the workbenches offer.
//!
//! The application's own commands are the document's queries and edits
//! (`doc.*`) and every command a key can run (`file.save`, `view.front`,
//! ...). A workbench's command runs in the workbench through
//! `Workbench::run_command`, with the same context its tools get, so the
//! host never needs to know what the command does. Every call's arguments
//! are checked against its spec before it runs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
};
use serde_json::{Value, json};
use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app::workbench_host::HookSite;
use crate::console::{self, LineKind};
use crate::ui::TreeItemId;
use crate::ui::keymap::{self, HostOutcome, HostState};

/// The application's own commands beyond the keyboard's.
fn doc_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("doc.info", "The document's name, file and unit")
            .returns("{name, file, unit, modified}"),
        CommandSpec::new("doc.bodies", "List the bodies")
            .returns("a list of {id, name, visible, features}"),
        CommandSpec::new("doc.features", "List the features in build order")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns("a list of {id, name, kind, body, visible, suppressed, error}"),
        CommandSpec::new("doc.feature", "A feature with its fields")
            .param("id", ParamKind::Id, "")
            .returns("{id, name, kind, body, visible, fields}"),
        CommandSpec::new("doc.selection", "What is selected")
            .returns("{item, body, feature}, each an id or nil"),
        CommandSpec::new(
            "doc.select",
            "Select a body or a feature, as a click on its row",
        )
        .param("id", ParamKind::Id, ""),
        CommandSpec::new("doc.new_body", "Make an empty body")
            .optional("name", ParamKind::String, "")
            .returns("the body's id"),
        CommandSpec::new("doc.rename", "Rename a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("name", ParamKind::String, ""),
        CommandSpec::new("doc.set_visible", "Show or hide a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("visible", ParamKind::Bool, ""),
        CommandSpec::new("doc.delete", "Delete a body or a feature").param("id", ParamKind::Id, ""),
        CommandSpec::new(
            "doc.rebuild",
            "Rebuild every solid that changed and wait for it",
        )
        .optional("timeout", ParamKind::Number, "Seconds to wait at most (60)")
        .returns("a list of {feature, error} for every feature that failed"),
        CommandSpec::new("doc.faces", "The faces of a body's solid, where it sits")
            .param("body", ParamKind::Id, "")
            .returns(
                "a list of {index, kind, point, area, normal?, axis?, radius?}: \
                 point lies on the face, normal is a flat face's outward one, \
                 axis a turned face's {point, direction}",
            ),
        CommandSpec::new(
            "doc.measure",
            "A body's volume, surface area, centre and bounds",
        )
        .param("body", ParamKind::Id, "")
        .returns("{volume, area, centre, min, max, approximate}"),
    ]
}

/// The commands a key can run that make sense without a key: all but
/// those that open a window of the interface's own.
fn key_commands() -> impl Iterator<Item = (CommandSpec, keymap::HostAction)> {
    use keymap::HostAction::*;
    keymap::host_actions()
        .filter(|(_, _, action)| {
            !matches!(
                action,
                Palette | Preferences | Console | Delete | ToggleVisibility | PivotAtCursor
            )
        })
        .map(|(id, label, action)| (with_file_args(CommandSpec::new(id, label), action), action))
}

/// A file command takes the file by name, to run without its dialog.
fn with_file_args(spec: CommandSpec, action: keymap::HostAction) -> CommandSpec {
    use keymap::HostAction::*;
    let path = |spec: CommandSpec, doc: &str| spec.optional("path", ParamKind::String, doc);
    match action {
        Open => path(spec, "The document to open; the dialog when left out")
            .returns("nothing; the document opens in its tab after the script"),
        SaveAs => path(spec, "Where to save; the dialog when left out"),
        Import => path(
            spec,
            "The STEP, IGES, STL, OBJ or 3MF file; the dialog when left out",
        )
        .returns("nothing; pc.doc.rebuild() waits for the import"),
        Export => path(spec, "Where to write; the dialog when left out")
            .optional(
                "format",
                ParamKind::String,
                "step, stl or 3mf; from the path's extension when left out",
            )
            .optional(
                "bodies",
                ParamKind::List,
                "The bodies to write; every visible one when left out",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "The mesh formats' distance to the true surface, mm (0.01)",
            )
            .returns("{path, written, skipped, triangles}"),
        _ => spec,
    }
}

/// The application's commands that are not a key's.
fn app_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("app.workbenches", "The workbenches, in the order they load")
            .returns("a list of {id, label, active}"),
        CommandSpec::new("app.workbench", "Switch to a workbench").param(
            "id",
            ParamKind::String,
            "As app.workbenches lists it",
        ),
        CommandSpec::new("app.tool", "Start a toolbar tool, as a click on it does").param(
            "id",
            ParamKind::String,
            "The tool's id, as Preferences › Keyboard lists it",
        ),
        CommandSpec::new("app.tools", "The tools of a workbench")
            .optional(
                "workbench",
                ParamKind::String,
                "The active one when left out",
            )
            .returns("a list of {id, label, keys}"),
    ]
}

/// The commands a script can call, by the application and then the
/// workbenches in registration order.
fn all_commands(app: &PrintCadApp) -> Vec<CommandSpec> {
    let mut out = doc_commands();
    out.extend(app_commands());
    out.extend(key_commands().map(|(spec, _)| spec));
    out.extend(app.registry.commands().into_iter().map(|(_, c)| c.clone()));
    out
}

/// What scripts run against: the application, for the length of one run.
struct AppHost<'a> {
    app: &'a mut PrintCadApp,
    event_loop: &'a ActiveEventLoop,
}

impl scripting::Host for AppHost<'_> {
    fn commands(&self) -> Vec<CommandSpec> {
        all_commands(self.app)
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        if let Some(spec) = doc_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.app.run_doc_command(id, &args, self.event_loop);
        }
        if let Some(spec) = app_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.app.run_app_command(id, &args);
        }
        if let Some((spec, action)) = key_commands().find(|(c, _)| c.id == id) {
            spec.check(&args)?;
            if Args(&args).has("path") {
                return self.app.run_file_command(action, &args);
            }
            return self.app.run_key_command(action, self.event_loop);
        }
        let Some((bench, spec)) = self.app.registry.command(id) else {
            return Err(CommandError::Unknown(id.to_string()));
        };
        spec.check(&args)?;
        let params = self.app.interaction_ctx_params();
        let (result, outcome) = self
            .app
            .with_workbench_ctx(&bench, params, |wb, ctx| wb.run_command(id, &args, ctx))
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        self.app.apply_hook_outcome(outcome, HookSite::Interaction);
        result
    }
}

impl PrintCadApp {
    /// Run one line typed in the console and show it with what it printed
    /// and came to.
    pub(crate) fn run_console_line(&mut self, line: &str, event_loop: &ActiveEventLoop) {
        console::push(LineKind::Input, line);
        let mut engine = self.scripts.take().unwrap_or_default();
        let out = engine.eval_line(
            line,
            &mut AppHost {
                app: self,
                event_loop,
            },
        );
        self.scripts = Some(engine);
        for printed in out.printed {
            console::push(LineKind::Printed, printed);
        }
        if let Some(value) = out.value {
            console::push(LineKind::Value, value);
        }
        if let Some(error) = out.error {
            console::push(LineKind::Error, error);
        }
        self.session.journal.label_next("Console");
        self.redraw_needed = true;
    }

    fn run_app_command(&mut self, id: &str, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        match id {
            "app.workbenches" => Ok(Value::Array(
                self.registry
                    .ids()
                    .iter()
                    .map(|bench| {
                        json!({
                            "id": bench.as_str(),
                            "label": self.registry.descriptor(bench).map(|d| d.label.clone()),
                            "active": *bench == self.session.active_workbench.0,
                        })
                    })
                    .collect(),
            )),
            "app.workbench" => {
                let bench = self.bench_arg(a.string("id")?)?;
                self.switch_workbench_for_flow(bench);
                Ok(Value::Null)
            }
            "app.tools" => {
                let bench = match a.opt_string("workbench")? {
                    Some(id) => self.bench_arg(id)?,
                    None => self.session.active_workbench.0.clone(),
                };
                let tools = self.registry.tools_for(&bench).unwrap_or_default();
                Ok(Value::Array(
                    tools
                        .iter()
                        .filter(|t| t.planned.is_none())
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "label": t.label,
                                "keys": t.shortcuts.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            })
                        })
                        .collect(),
                ))
            }
            "app.tool" => {
                let id = a.string("id")?;
                let bench = self
                    .registry
                    .ids()
                    .iter()
                    .find(|bench| {
                        self.registry.tools_for(bench).is_ok_and(|tools| {
                            tools.iter().any(|t| t.id == id && t.planned.is_none())
                        })
                    })
                    .cloned()
                    .ok_or_else(|| CommandError::bad("id", "is not a tool"))?;
                if bench != self.session.active_workbench.0 {
                    self.switch_workbench_for_flow(bench.clone());
                }
                let tools = self.registry.tools_for(&bench).unwrap_or_default().to_vec();
                if let Some(tool) = tools.iter().find(|t| t.id == id) {
                    crate::ui::toolbar::activate_tool(
                        &mut self.session.active_tool,
                        &tools,
                        tool,
                        id,
                    );
                }
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }

    fn bench_arg(&self, id: &str) -> Result<core_document::WorkbenchId, CommandError> {
        self.registry
            .ids()
            .iter()
            .find(|b| b.as_str() == id)
            .cloned()
            .ok_or_else(|| {
                CommandError::bad("id", "is not a workbench; app.workbenches lists them")
            })
    }

    /// A file command given its file: no dialog.
    fn run_file_command(
        &mut self,
        action: keymap::HostAction,
        args: &CommandArgs,
    ) -> CommandResult {
        use keymap::HostAction::*;
        let a = Args(args);
        let path = std::path::PathBuf::from(a.string("path")?);
        match action {
            Open => {
                self.open_document_at(path);
                Ok(Value::Null)
            }
            SaveAs => {
                self.save_document_at(&path)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(Value::Null)
            }
            Import => {
                if !path.is_file() {
                    return Err(CommandError::bad("path", "is not a file"));
                }
                let detail = self.last_step_import_detail.clone();
                self.import_step_at(&path, detail);
                Ok(Value::Null)
            }
            Export => {
                let format = match a.opt_string("format")? {
                    Some(name) => match name.to_ascii_lowercase().as_str() {
                        "step" | "stp" => kernel_ogeom::export::ExportFormat::Step,
                        "stl" => kernel_ogeom::export::ExportFormat::Stl,
                        "3mf" => kernel_ogeom::export::ExportFormat::ThreeMf,
                        _ => return Err(CommandError::bad("format", "must be step, stl or 3mf")),
                    },
                    None => {
                        kernel_ogeom::export::ExportFormat::of_path(&path).ok_or_else(|| {
                            CommandError::bad(
                                "format",
                                "is needed when the path has no .step, .stl or .3mf",
                            )
                        })?
                    }
                };
                let bodies = match args.get("bodies") {
                    Some(Value::Array(list)) => Some(
                        list.iter()
                            .map(|v| {
                                v.as_str()
                                    .and_then(|s| Uuid::parse_str(s).ok())
                                    .map(BodyId)
                                    .ok_or_else(|| {
                                        CommandError::bad("bodies", "must be a list of body ids")
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    _ => None,
                };
                let tolerance = a.opt_number("tolerance")?.map(|t| t as f32);
                let (path, exported) = self
                    .export_now(path, format, bodies, tolerance)
                    .map_err(CommandError::failed)?;
                Ok(json!({
                    "path": path.display().to_string(),
                    "written": exported.written,
                    "skipped": exported.skipped,
                    "triangles": exported.triangles,
                }))
            }
            _ => Err(CommandError::failed("this command takes no file")),
        }
    }

    fn run_key_command(
        &mut self,
        action: keymap::HostAction,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let state = HostState {
            active_tab: Some(self.session.tab),
            section_on: self.session.section.is_some(),
            document: &self.session.document,
            tree_selection: self.session.tree_selection,
            editing: false,
        };
        match keymap::host_outcome(action, &state) {
            HostOutcome::Command(command) => {
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            _ => Err(CommandError::failed("this command is not available here")),
        }
    }

    fn run_doc_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let a = Args(args);
        let document = &self.session.document;
        match id {
            "doc.info" => Ok(json!({
                "name": document.name(),
                "file": self.session.current_file.as_ref().map(|p| p.display().to_string()),
                "unit": document.display_unit().short_label(),
                "modified": document.metadata().dirty(),
            })),
            "doc.bodies" => Ok(Value::Array(
                document
                    .bodies()
                    .iter()
                    .map(|b| {
                        json!({
                            "id": b.id.0.to_string(),
                            "name": b.name,
                            "visible": !b.hidden,
                            "features": features_in_order(document, Some(b.id))
                                .iter()
                                .map(|n| n.id.0.to_string())
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            )),
            "doc.features" => {
                let body = a.opt_id("body")?.map(BodyId);
                Ok(Value::Array(
                    features_in_order(document, None)
                        .into_iter()
                        .filter(|n| body.is_none() || n.body == body)
                        .map(|n| {
                            json!({
                                "id": n.id.0.to_string(),
                                "name": n.name,
                                "kind": self.kind_of(n),
                                "body": n.body.map(|b| b.0.to_string()),
                                "visible": n.visible,
                                "suppressed": n.suppressed,
                                "error": n.error,
                            })
                        })
                        .collect(),
                ))
            }
            "doc.feature" => {
                let node = document
                    .get_feature_meta(FeatureId(a.id("id")?))
                    .ok_or_else(|| CommandError::bad("id", "is not a feature of this document"))?;
                Ok(json!({
                    "id": node.id.0.to_string(),
                    "name": node.name,
                    "kind": self.kind_of(node),
                    "body": node.body.map(|b| b.0.to_string()),
                    "visible": node.visible,
                    "fields": node.data,
                }))
            }
            "doc.selection" => Ok(json!({
                "item": self.session.tree_selection.and_then(item_id).map(|u| u.to_string()),
                "body": self.session.active_body_id.map(|b| b.0.to_string()),
                "feature": self.session.active_document_object.map(|f| f.0.to_string()),
            })),
            "doc.select" => {
                let item = self.tree_item(a.id("id")?)?;
                self.apply_tree_selection(item);
                Ok(Value::Null)
            }
            "doc.new_body" => {
                let body = self.session.document.create_body(None);
                if let Some(name) = a.opt_string("name")? {
                    self.session.document.rename_body(body, name);
                }
                Ok(json!(body.0.to_string()))
            }
            "doc.rename" => {
                let name = a.string("name")?.to_string();
                match self.tree_item(a.id("id")?)? {
                    TreeItemId::Body(body) => self.session.document.rename_body(body, name),
                    TreeItemId::Feature(feature) => {
                        self.session.document.rename_feature(feature, name)
                    }
                    _ => return Err(CommandError::bad("id", "cannot be renamed")),
                }
                Ok(Value::Null)
            }
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                let command = match self.tree_item(a.id("id")?)? {
                    TreeItemId::Body(body) => {
                        crate::ui::UiCommand::SetBodyVisible { body, visible }
                    }
                    TreeItemId::Feature(feature) => crate::ui::UiCommand::TreeFeature {
                        feature,
                        command: crate::ui::TreeFeatureCommand::SetVisible(visible),
                    },
                    TreeItemId::ImportedObject(node) => {
                        crate::ui::UiCommand::SetImportedVisibility { node, visible }
                    }
                    TreeItemId::DocumentRoot => {
                        return Err(CommandError::bad("id", "cannot be hidden"));
                    }
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = self.tree_item(a.id("id")?)?;
                self.apply_ui_commands(
                    vec![crate::ui::UiCommand::DeleteTreeItem(item)],
                    event_loop,
                );
                Ok(Value::Null)
            }
            "doc.rebuild" => {
                let timeout = a.opt_number("timeout")?.unwrap_or(60.0);
                self.rebuild_and_wait(std::time::Duration::from_secs_f64(timeout.max(0.0)))
            }
            "doc.faces" => {
                let body = self.body_arg(&a)?;
                let geometry = self
                    .session
                    .document
                    .imported_geometry(body)
                    .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
                Ok(faces_of(&geometry.mesh))
            }
            "doc.measure" => {
                let body = self.body_arg(&a)?;
                let document = &self.session.document;
                let (min, max) = document
                    .imported_geometry(body)
                    .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                    .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
                let blob = document.imported_brep_blob_arc(body).ok_or_else(|| {
                    CommandError::failed("the body is a mesh; it has no solid to measure")
                })?;
                let props = kernel_ogeom::OgeomKernel::new()
                    .physical_properties(&blob)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let c = props.centre_mm.map(|v| v as f32);
                let centre = document.body_placement(body).point(c);
                Ok(json!({
                    "volume": props.volume_mm3,
                    "area": props.area_mm2,
                    "centre": centre,
                    "min": min,
                    "max": max,
                    "approximate": props.approximate,
                }))
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }

    fn body_arg(&self, a: &Args) -> Result<BodyId, CommandError> {
        let body = BodyId(a.id("body")?);
        if self.session.document.bodies().iter().any(|b| b.id == body) {
            Ok(body)
        } else {
            Err(CommandError::bad("body", "is not a body of this document"))
        }
    }

    /// Send every changed body to the kernel, as a frame would, and wait
    /// for the answers: again while answers leave more to rebuild.
    fn rebuild_and_wait(&mut self, timeout: std::time::Duration) -> CommandResult {
        let started = std::time::Instant::now();
        loop {
            self.drive_part_recompute();
            self.drive_shape_repairs();
            if self.kernel_worker.in_flight() == 0 {
                break;
            }
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(CommandError::failed(format!(
                    "the kernel was still working after {} s",
                    timeout.as_secs_f32()
                )));
            }
            if self.kernel_worker.wait(left) {
                self.drain_kernel_responses();
            }
        }
        self.redraw_needed = true;
        let errors: Vec<Value> = features_in_order(&self.session.document, None)
            .into_iter()
            .filter_map(|n| {
                n.error
                    .as_ref()
                    .map(|e| json!({"feature": n.id.0.to_string(), "name": n.name, "error": e}))
            })
            .collect();
        Ok(Value::Array(errors))
    }

    /// What kind of feature `node` is, as its workbench names it.
    fn kind_of(&self, node: &core_document::FeatureNode) -> String {
        self.registry
            .feature_info(node)
            .map(|info| info.kind_label)
            .unwrap_or_else(|| node.workbench_id.as_str().to_string())
    }

    /// The tree row an id names: a body, a feature or an imported part.
    fn tree_item(&self, id: Uuid) -> Result<TreeItemId, CommandError> {
        let document = &self.session.document;
        if document.bodies().iter().any(|b| b.id.0 == id) {
            Ok(TreeItemId::Body(BodyId(id)))
        } else if document.get_feature_meta(FeatureId(id)).is_some() {
            Ok(TreeItemId::Feature(FeatureId(id)))
        } else if document.imported_object(id).is_some() {
            Ok(TreeItemId::ImportedObject(id))
        } else {
            Err(CommandError::bad("id", "is not in this document"))
        }
    }
}

/// The features of `body` (or all of them) in build order.
fn features_in_order(
    document: &core_document::Document,
    body: Option<BodyId>,
) -> Vec<&core_document::FeatureNode> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .filter(|n| body.is_none() || n.body == body)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
}

/// Each face of a mesh: what surface it is, a point on it, its area, and a
/// flat face's normal or a turned face's axis.
fn faces_of(mesh: &kernel_api::TriMesh) -> Value {
    let count = mesh.face_surfaces.len().max(
        mesh.faces
            .iter()
            .map(|f| *f as usize + 1)
            .max()
            .unwrap_or(0),
    );
    // Per face: area, the area-weighted centre, and the triangle centre
    // nearest it (a point that is on the face even when it curves).
    let mut area = vec![0.0f32; count];
    let mut centre = vec![[0.0f32; 3]; count];
    let tri = |t: usize| {
        let p = |i: usize| glam::Vec3::from(mesh.positions[mesh.indices[3 * t + i] as usize]);
        (p(0), p(1), p(2))
    };
    for (t, face) in mesh.faces.iter().enumerate() {
        let (a, b, c) = tri(t);
        let w = (b - a).cross(c - a).length() / 2.0;
        let f = *face as usize;
        area[f] += w;
        let m = (a + b + c) / 3.0;
        for i in 0..3 {
            centre[f][i] += m[i] * w;
        }
    }
    let mut on_face = vec![None::<(f32, glam::Vec3)>; count];
    for (t, face) in mesh.faces.iter().enumerate() {
        let f = *face as usize;
        if area[f] <= 0.0 {
            continue;
        }
        let target = glam::Vec3::from(centre[f]) / area[f];
        let (a, b, c) = tri(t);
        let m = (a + b + c) / 3.0;
        let d = m.distance(target);
        if on_face[f].is_none_or(|(best, _)| d < best) {
            on_face[f] = Some((d, m));
        }
    }
    let list = (0..count)
        .filter_map(|f| {
            let (_, point) = on_face[f]?;
            let surface = mesh.face_surfaces.get(f).copied().unwrap_or_default();
            let kind = match surface {
                kernel_api::FaceSurface::Plane { .. } => "plane",
                kernel_api::FaceSurface::Cylinder { .. } => "cylinder",
                kernel_api::FaceSurface::Cone { .. } => "cone",
                kernel_api::FaceSurface::Sphere { .. } => "sphere",
                kernel_api::FaceSurface::Torus { .. } => "torus",
                kernel_api::FaceSurface::Other => "other",
            };
            let mut out = json!({
                "index": f,
                "kind": kind,
                "point": point.to_array(),
                "area": area[f],
            });
            match surface {
                kernel_api::FaceSurface::Plane { normal, .. } => out["normal"] = json!(normal),
                kernel_api::FaceSurface::Cylinder { radius, .. }
                | kernel_api::FaceSurface::Sphere { radius, .. } => out["radius"] = json!(radius),
                _ => {}
            }
            if let Some((p, d)) = surface.axis() {
                out["axis"] = json!({"point": p, "direction": d});
            }
            Some(out)
        })
        .collect();
    Value::Array(list)
}

fn item_id(item: TreeItemId) -> Option<Uuid> {
    match item {
        TreeItemId::Body(b) => Some(b.0),
        TreeItemId::Feature(f) => Some(f.0),
        TreeItemId::ImportedObject(n) => Some(n),
        TreeItemId::DocumentRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_listed_with_a_point_on_each_and_what_surface_it_is() {
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
                [0.0, 0.0, 5.0],
                [1.0, 0.0, 5.0],
                [0.0, 1.0, 5.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
            faces: vec![0, 0, 1],
            face_surfaces: vec![
                kernel_api::FaceSurface::Plane {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, -1.0],
                },
                kernel_api::FaceSurface::Cylinder {
                    origin: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 3.0,
                },
            ],
            ..Default::default()
        };
        let faces = faces_of(&mesh);
        let faces = faces.as_array().unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0]["kind"], "plane");
        assert_eq!(faces[0]["normal"], json!([0.0, 0.0, -1.0]));
        assert!((faces[0]["area"].as_f64().unwrap() - 100.0).abs() < 1e-3);
        assert_eq!(faces[0]["point"][2], json!(0.0));
        assert_eq!(faces[1]["kind"], "cylinder");
        assert_eq!(faces[1]["axis"]["direction"], json!([0.0, 0.0, 1.0]));
        assert_eq!(faces[1]["radius"], json!(3.0));
    }

    #[test]
    fn the_application_s_command_ids_are_unique() {
        let mut ids: Vec<String> = doc_commands().into_iter().map(|c| c.id).collect();
        ids.extend(app_commands().into_iter().map(|c| c.id));
        ids.extend(key_commands().map(|(c, _)| c.id));
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
