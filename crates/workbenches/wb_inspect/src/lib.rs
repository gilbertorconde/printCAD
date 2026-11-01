//! The Inspection workbench: measuring a scan. Its first tools fit a plane,
//! sphere, cylinder, cone, torus, circle or line to the part of a scan the
//! user marked (Mark surface…), by orthogonal least squares with points
//! beyond three sigmas left out, and keep the result as an element under
//! the scan: its size, its sigma, its form error, and the lines that draw
//! it no larger than measured.

pub mod element;

use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureInfo,
    FeatureNode, InputResult, ParamKind, PassiveGeometry, PassiveTint, ToolDescriptor,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use element::{Element, Fitted};
use kernel_api::{FitKind, FitRequest};
use serde_json::{Value, json};

/// The workbench's id.
pub const ID: &str = "wb.inspect";

/// Points farther from the fit than this many sigmas are left out.
const DEFAULT_CUTOFF: f64 = 3.0;

const FIT_COMMAND: &str = "inspect.fit";
const REPORT_COMMAND: &str = "inspect.report";

#[derive(Default)]
pub struct InspectionWorkbench;

fn tool_id(kind: FitKind) -> String {
    format!("inspect.fit_{}", element::kind_word(kind).to_lowercase())
}

fn kind_of_tool(id: &str) -> Option<FitKind> {
    FitKind::ALL.into_iter().find(|k| tool_id(*k) == id)
}

fn kind_named(name: &str) -> Option<FitKind> {
    FitKind::ALL
        .into_iter()
        .find(|k| element::kind_word(*k).eq_ignore_ascii_case(name))
}

/// Fit `kind` to the marked part of `scan` and keep it as an element.
fn fit_marked(
    ctx: &mut WorkbenchRuntimeContext,
    kind: FitKind,
    cutoff: Option<f64>,
) -> Result<(core_document::FeatureId, Element), String> {
    let marking = ctx
        .marking
        .clone()
        .ok_or("nothing is marked: mark part of a scan first (Mark surface… in its menu)")?;
    let scan = marking.body;
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or("the marked scan has no mesh")?;
    let region = element::runs(&marking.vertices);
    let picked: Vec<usize> = element::vertices(&region)
        .map(|v| v as usize)
        .filter(|v| *v < mesh.positions.len())
        .collect();
    let point = |v: usize| mesh.positions[v].map(f64::from);
    let points: Vec<[f64; 3]> = picked.iter().map(|v| point(*v)).collect();
    let normals = if mesh.normals.len() == mesh.positions.len() {
        picked
            .iter()
            .map(|v| mesh.normals[*v].map(f64::from))
            .collect()
    } else {
        Vec::new()
    };
    let kernel = ctx.kernel.ok_or("no kernel to fit with")?;
    let answer = kernel
        .fit(&FitRequest {
            kind,
            points: points.clone(),
            normals,
            sigma_cutoff: cutoff,
            fixed_radius: None,
        })
        .map_err(|e| e.to_string())?;
    let used: Vec<[f64; 3]> = points
        .iter()
        .zip(&answer.used)
        .filter(|(_, u)| **u)
        .map(|(p, _)| *p)
        .collect();
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == element::KIND && n.body == Some(scan))
        .count()
        + 1;
    let name = format!("{} {number}", element::kind_word(kind));
    let element = Element {
        name: name.clone(),
        scan,
        kind,
        cutoff,
        region,
        revision: marking.revision,
        result: Fitted {
            span: element::span_of(&answer.shape, &used),
            shape: answer.shape,
            sigma: answer.sigma,
            form: answer.form,
            used: used.len(),
            points: points.len(),
        },
    };
    let id = ctx
        .document
        .add_feature_in_body(element.clone(), name, Some(scan))
        .map_err(|e| e.to_string())?;
    Ok((id, element))
}

fn answer_of(id: core_document::FeatureId, e: &Element) -> Value {
    let fitted = &e.result;
    json!({
        "element": id.0.to_string(),
        "name": e.name,
        "kind": element::kind_word(e.kind).to_lowercase(),
        "shape": fitted.shape,
        "sigma": fitted.sigma,
        "form": fitted.form.1 - fitted.form.0,
        "diameters": fitted.diameters().map(|(lo, hi)| json!({"min": lo, "max": hi})),
        "used": fitted.used,
        "points": fitted.points,
    })
}

impl core_document::Workbench for InspectionWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            ID,
            "Inspection",
            "Measure a scan: fit shapes to its surface and read their size and form",
        )
        .icon("measure")
        .feature_kinds([element::KIND])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        for kind in FitKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(
                    tool_id(kind),
                    format!("Fit {}", element::kind_word(kind).to_lowercase()),
                    Some("fit"),
                )
                .icon(element::icon(kind)),
            );
        }
        context.register_command(
            CommandSpec::new(
                FIT_COMMAND,
                "Fit a shape to the marked part of a scan and keep it as an element",
            )
            .param(
                "kind",
                ParamKind::String,
                "plane, sphere, cylinder, cone, torus, circle or line",
            )
            .optional(
                "cutoff",
                ParamKind::Number,
                "Leave out points farther than this many sigmas, fitting again until the \
                 set kept stops changing; 3 when left out, 0 keeps every point",
            )
            .returns(
                "{element, name, kind, shape, sigma, form, diameters, used, points}: in mm, \
                 the shape in the scan's own frame",
            )
            .note(
                "It fits what is marked (`pc.scan.mark`, or Mark surface… in the scan's \
                 menu), by orthogonal least squares. `sigma` is the RMS distance of the points \
                 used from the shape, `form` the width of the band they fill, `diameters` the \
                 smallest and largest a round shape reaches.",
            ),
        );
        context.register_command(
            CommandSpec::new(REPORT_COMMAND, "Every element of the inspection, measured")
                .optional("scan", ParamKind::Id, "Only this scan's elements")
                .returns("a list as inspect.fit answers each element")
                .read_only(),
        );
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        let Ok(element) = Element::from_json(&node.data) else {
            return FeatureInfo::fallback(node);
        };
        FeatureInfo {
            icon: element::icon(element.kind),
            kind_label: format!(
                "{} · {}",
                element::kind_word(element.kind),
                element.result.summary()
            ),
            family_label: "Inspection element".to_string(),
            builds_solid: false,
        }
    }

    fn passive_geometry(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        _node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        let data = document.feature_values(id)?;
        let element = Element::from_json(data).ok()?;
        Some(PassiveGeometry {
            mesh: element.result.lines(),
            revision: core_document::data_revision(data),
            tint: PassiveTint::External,
            region: None,
        })
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        kind_of_tool(tool_id).is_none() || ctx.marking.is_some()
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        let (WorkbenchInputEvent::ToolActivated, Some(kind)) =
            (event, active_tool.and_then(kind_of_tool))
        else {
            return InputResult::ignored();
        };
        match fit_marked(ctx, kind, Some(DEFAULT_CUTOFF)) {
            Ok((id, element)) => {
                let mut args = CommandArgs::new();
                args.insert(
                    "kind".into(),
                    json!(element::kind_word(kind).to_lowercase()),
                );
                ctx.record(FIT_COMMAND, args, answer_of(id, &element));
                ctx.log_info(format!("{}: {}", element.name, element.result.summary()));
            }
            Err(why) => ctx.log_warn(why),
        }
        InputResult::consumed()
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let a = core_document::Args(args);
        match id {
            FIT_COMMAND => {
                let kind = kind_named(a.string("kind")?).ok_or_else(|| {
                    CommandError::bad(
                        "kind",
                        "is plane, sphere, cylinder, cone, torus, circle or line",
                    )
                })?;
                let cutoff = match a.opt_number("cutoff")? {
                    Some(k) if k <= 0.0 => None,
                    Some(k) => Some(k),
                    None => Some(DEFAULT_CUTOFF),
                };
                let (id, element) = fit_marked(ctx, kind, cutoff).map_err(CommandError::failed)?;
                Ok(answer_of(id, &element))
            }
            REPORT_COMMAND => {
                let only = a.0.get("scan").and(a.id("scan").ok()).map(BodyId);
                let document = &*ctx.document;
                Ok(Value::Array(
                    document
                        .feature_tree()
                        .all_nodes()
                        .filter(|(_, n)| n.workbench_id.as_str() == element::KIND)
                        .filter(|(_, n)| only.is_none() || n.body == only)
                        .filter_map(|(id, _)| {
                            let data = document.feature_values(*id)?;
                            Some(answer_of(*id, &Element::from_json(data).ok()?))
                        })
                        .collect(),
                ))
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Workbench;
    use std::sync::Arc;

    /// A kernel that fits every request as a unit sphere and keeps all
    /// points but the last.
    struct UnitSphere;

    impl kernel_api::KernelQueries for UnitSphere {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("project_edge".into()))
        }

        fn fit(&self, request: &FitRequest) -> kernel_api::KernelResult<kernel_api::FitAnswer> {
            let n = request.points.len();
            Ok(kernel_api::FitAnswer {
                shape: kernel_api::FittedShape::Sphere {
                    centre: [0.0; 3],
                    radius: 1.0,
                },
                sigma: 0.01,
                form: (-0.02, 0.03),
                used: (0..n).map(|i| i + 1 < n).collect(),
                residuals: vec![0.0; n],
                converged: true,
            })
        }
    }

    /// Every tool names an icon the set has. A fit takes the marked part of
    /// a scan, keeps it as an element under the scan with the vertices it
    /// rests on and what the kernel made of them, records itself as a
    /// command, and the report lists it; with nothing marked the tool says
    /// so and makes nothing.
    #[test]
    fn a_fit_of_the_marked_surface_becomes_an_element() {
        let mut context = WorkbenchContext::default();
        InspectionWorkbench.configure(&mut context);
        assert_eq!(context.tools().len(), FitKind::ALL.len());
        for tool in context.tools() {
            let icon = tool.icon.expect("an icon");
            assert!(ui_kit::icon::exists(icon), "{icon}");
        }
        assert!(ui_kit::icon::exists("measure"));

        let mut document = Document::new("Scan");
        let scan = document.create_body(Some("Scan".into()));
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 0.0],
            ],
            normals: vec![
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 0.0],
            ],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        document.set_imported_geometry(
            scan,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let mut bench = InspectionWorkbench;
        let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&UnitSphere);
        let tool = tool_id(FitKind::Sphere);
        assert!(!bench.is_tool_enabled(&tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
        let outcome = core_document::HookOutcome::take(&mut ctx);
        assert!(outcome.recorded.is_empty(), "nothing marked, nothing made");

        ctx.marking = Some(core_document::MarkedSurface {
            body: scan,
            revision: 0,
            vertices: Arc::new(vec![true, true, false, true]),
        });
        assert!(bench.is_tool_enabled(&tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
        assert_eq!(
            core_document::HookOutcome::take(&mut ctx).recorded[0].id,
            FIT_COMMAND
        );
        let report = bench
            .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
            .unwrap();
        let first = &report[0];
        assert_eq!(first["name"], "Sphere 1");
        assert_eq!(
            (first["points"].clone(), first["used"].clone()),
            (json!(3), json!(2))
        );
        assert!((first["form"].as_f64().unwrap() - 0.05).abs() < 1e-12);
        let id = core_document::FeatureId(
            uuid::Uuid::parse_str(first["element"].as_str().unwrap()).unwrap(),
        );
        let data = ctx.document.feature_values(id).unwrap();
        let element = Element::from_json(data).unwrap();
        assert_eq!(
            (element.scan, element.region.clone()),
            (scan, vec![[0, 2], [3, 4]])
        );
        assert_eq!(element.result.lines().edges.len() / 2, 3 * 48);
    }
}
