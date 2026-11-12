//! The Inspection workbench: measuring a scan. Its first tools fit a plane,
//! sphere, cylinder, cone, torus, circle or line to the part of a scan the
//! user marked (Mark surface…), by orthogonal least squares with points
//! beyond three sigmas left out, and keep the result as an element under
//! the scan: its size, its sigma, its form error, and the lines that draw
//! it no larger than measured. Dimensions between elements read
//! distances, angles and diameters off them, each with a nominal and a
//! band it passes or fails.

// Without the egui feature the crate is the headless subset its dependents'
// tests and the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code, unused_imports))]

pub mod align;
mod align_commands;
mod best_fit;
pub mod clearance;
mod commands;
mod construction_export;
use construction_export::write_construction;
pub mod compare;
pub mod compensation;
pub mod construction;
pub mod dimension;
pub mod element;
pub mod element_map;
pub mod export;
pub mod extract;
mod features;
mod fitting;
use fitting::{answer_of, fit_marked, grow_at, kind_named, method_arg};
pub mod hotspots;
mod input;
mod landing;
use landing::{best_fit_how, record_alignment};
mod making;
use making::{
    add_construction, add_dimension, add_tolerance, built_at, construction_answer,
    dimension_answer, dimension_at, element_at, element_solid, ids_in, numbered, selected_elements,
    tolerance_answer, tolerance_at,
};
mod mapping;
use mapping::{
    add_clearance, add_comparison, add_overhang, add_thickness, comparison_values,
    element_map_inputs, longest_side,
};
pub mod maps;
pub mod overhang;
mod picks;
pub mod pins;
mod pmi;
mod registration;
use registration::kind_of_tool;
pub mod report;
use report::report_of;
pub mod section;
mod sectioning;
use sectioning::{section_answer, section_plane, shadow_answer, shadows_on};
pub mod shadow;
pub mod thickness;
pub mod tolerance;
mod tolerance_read;
mod tolerance_task;
mod vec;
mod viewport_out;
pub mod volume;

use compare::Comparison;
use construction::Made;
use core_document::{
    BodyId, CommandArgs, CommandResult, Document, FeatureInfo, FeatureNode, InputResult,
    PassiveGeometry, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use dimension::DimKind;
use element::Element;
#[cfg(test)]
use element::Fitted;
use glam::DVec3;
use kernel_api::FitKind;
#[cfg(test)]
use kernel_api::FitRequest;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

/// The workbench's id.
pub const ID: &str = "wb.inspect";

/// Points farther from the fit than this many sigmas are left out.
const DEFAULT_CUTOFF: f64 = 3.0;

const FIT_COMMAND: &str = "inspect.fit";
const REPORT_COMMAND: &str = "inspect.report";
const DIMENSION_COMMAND: &str = "inspect.dimension";
const CONSTRUCT_COMMAND: &str = "inspect.construct";
const GROW_COMMAND: &str = "inspect.grow";
const COMPARE_COMMAND: &str = "inspect.compare";
const COMPARE_TOOL: &str = "inspect.compare";
const BEST_FIT: &str = "inspect.best_fit";
const RESET_ALIGNMENT: &str = "inspect.reset_alignment";
const MOVE_COMMAND: &str = "inspect.move";
const HOT_SPOTS: &str = "inspect.hot_spots";
const VOLUME_ERRORS: &str = "inspect.volume_errors";
const THICKNESS: &str = "inspect.walls";
const OVERHANG: &str = "inspect.overhang";
const CLEARANCE: &str = "inspect.clearance";
const ELEMENT_SOLID: &str = "inspect.element_solid";
const EXTRACT: &str = "inspect.extract";
const TOLERANCE: &str = "inspect.tolerance";
const SAVE_REPORT: &str = "inspect.save_report";
const ELEMENT_MAP: &str = "inspect.element_map";
const PIN: &str = "inspect.pin";
const SECTION: &str = "inspect.section";
const COMPENSATION: &str = "inspect.compensation";
const EXPORT_STEP: &str = "inspect.export_step";
const SAVE_STEP: &str = "inspect.save_step";
const EXPORT_IGES: &str = "inspect.export_iges";
const SAVE_IGES: &str = "inspect.save_iges";
const SHADOW: &str = "inspect.shadow";

/// How far from the cursor a reading looks for a scan point, mm.
const READING_REACH: f64 = 2.0;

const MAP_EXPORT: &str = "inspect.map_export";

/// The fewest vertices a labelled hot spot has.
const SPOT_LEAST: usize = 3;

/// What the bench holds for the document on screen, moved with its tab.
#[derive(Default)]
struct Tab {
    /// The fit a click on a scan makes, while nothing is marked.
    armed: Option<FitKind>,
    /// A click on a mapped scan pins a reading there.
    pinning: bool,
    /// The clicks of an element in progress, pinned until it is made.
    picks: picks::Picks,
    tolerance_task: Option<tolerance_task::Task>,
    /// The best fit the toolbar started, while it runs and until applied.
    fitting: best_fit::Fitting,
    /// Every map's values, measured away from the window.
    maps: maps::Maps,
    tolerance_readings: tolerance_read::Readings,
}

#[derive(Default)]
pub struct InspectionWorkbench {
    tab: Tab,
    /// Every section's cuts, by what they were cut from.
    cuts: section::Cuts,
    /// The kernel the host hands the bench, kept for drawing sections.
    kernel: std::sync::OnceLock<&'static dyn kernel_api::KernelQueries>,
}

impl core_document::Workbench for InspectionWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            ID,
            "Inspection",
            "Measure a scan: fit shapes to its surface and read their size and form",
        )
        .icon("measure")
        .feature_kinds([
            shadow::KIND,
            section::KIND,
            pins::KIND,
            element_map::KIND,
            tolerance::KIND,
            thickness::KIND,
            overhang::KIND,
            clearance::KIND,
            element::KIND,
            dimension::KIND,
            construction::KIND,
            compare::KIND,
            align::KIND,
        ])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        self.register_tools(context);
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        self.info_of(node)
    }

    fn passive_geometry(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        self.passive_of(document, id, node)
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        self.tool_enabled(tool_id, ctx)
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        (tool_id == PIN && self.tab.pinning)
            || (tool_id == BEST_FIT && self.tab.fitting.running())
            || (tool_id == align_commands::POINTS && self.tab.picks.aligning)
            || (self.tab.armed.is_some() && kind_of_tool(tool_id) == self.tab.armed)
    }

    fn parameters(&self, node: &FeatureNode) -> Vec<core_document::evaluate::Parameter> {
        self.parameters_of(node)
    }

    fn delete_feature(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: core_document::FeatureId,
    ) -> bool {
        self.delete_with_dependents(ctx, id)
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: core_document::FeatureId) {
        if let Some(node) = ctx.document.feature_tree().get_node(id)
            && node.workbench_id.as_str() == tolerance::KIND
            && let Ok(t) = tolerance::Tolerance::from_json(&node.data)
        {
            self.tab.tolerance_task = Some(tolerance_task::Task {
                id,
                opening: Some(t),
            });
        }
    }

    fn editing_feature(&self) -> Option<core_document::FeatureId> {
        self.tab.tolerance_task.as_ref().map(|t| t.id)
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        Some(&mut self.tab)
    }

    fn task(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        self.tab.tolerance_task.as_ref()?;
        Some(core_document::TaskInfo {
            title: "Geometric tolerance".into(),
            icon: "constraint-perpendicular",
            confirmable: true,
            stepwise: false,
            ..Default::default()
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
        let Some(task) = self.tab.tolerance_task.as_ref() else {
            return TaskOutcome::Open;
        };
        let id = task.id;
        let Some(data) = ctx.document.get_feature_data(id) else {
            self.tab.tolerance_task = None;
            return TaskOutcome::Cancelled;
        };
        // Cancel's edits are taken back by the host.
        if request.cancel {
            self.tab.tolerance_task = None;
            return TaskOutcome::Cancelled;
        }
        if tolerance::Tolerance::from_json(data).is_err() {
            return TaskOutcome::Open;
        }
        let widgets = self.task_widgets(ctx);
        let out = bench_panel::show(
            ui,
            egui::Id::new("tolerance_task"),
            &widgets,
            Some(&*ctx.document),
        );
        let by_formula = out.write_formulas(ctx);
        for event in out.events_besides(&by_formula) {
            self.task_event(ctx, event);
        }
        if request.accept
            && let Some((id, t)) = self.tolerance_in_task(ctx)
            && let Some(task) = &self.tab.tolerance_task
        {
            let reading = self
                .tab
                .tolerance_readings
                .read(ctx.document, id, &t, ctx.kernel, false);
            if reading.is_ok() {
                let args = tolerance_task::args(&t, task.opening.as_ref().map(|_| id));
                ctx.record(TOLERANCE, args, tolerance_answer(id, &t, &reading));
                self.tab.tolerance_task = None;
                return TaskOutcome::Accepted {
                    label: "Geometric tolerance".into(),
                };
            }
        }
        TaskOutcome::Open
    }

    fn busy(&self) -> bool {
        self.tab.maps.busy() || self.tab.tolerance_readings.busy() || self.tab.fitting.running()
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(kernel) = ctx.kernel {
            let _ = self.kernel.set(kernel);
        }
        self.land_best_fit(ctx);
    }

    fn on_background_frame(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.land_best_fit(ctx);
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            scalar_maps: self.scalar_maps(ctx),
            status: self.status_items(ctx),
            faded: self.faded_bodies(ctx),
            ..Default::default()
        }
    }

    /// A shown comparison's reference draws see-through, so the scan's map
    /// shows where the reference would cover it.
    /// A shadow's outline follows the scan's mesh, where it stands and the
    /// shadow's settings.
    fn derive_on_geometry(
        &self,
        node: &FeatureNode,
        values: &mut Value,
        document: &Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        if node.workbench_id.as_str() != shadow::KIND {
            return false;
        }
        let (Some(kernel), Ok(s)) = (kernel, shadow::Shadow::from_json(values)) else {
            return false;
        };
        if s.made
            .as_ref()
            .is_some_and(|m| m.key == shadow::key(document, &s))
        {
            return false;
        }
        let made = shadow::make(document, kernel, &s);
        values["made"] = serde_json::to_value(made).unwrap_or_default();
        true
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<core_document::RebuildJob> {
        let dirty: Vec<BodyId> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND && n.dirty)
            .filter_map(|(_, n)| n.body)
            .collect();
        let mut jobs = Vec::new();
        for body in dirty {
            if jobs
                .iter()
                .any(|j: &core_document::RebuildJob| j.body == body)
            {
                continue;
            }
            let mut plan = core_document::BuildPlan::default();
            for (id, s) in shadows_on(document, body) {
                document.clear_feature_dirty(id);
                let ops = shadow::ops(&s);
                plan.op_features.extend(ops.iter().map(|_| id));
                plan.ops.extend(ops);
            }
            jobs.push(core_document::RebuildJob {
                body,
                plan: Ok(plan),
            });
        }
        jobs
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        for (id, _) in shadows_on(document, body) {
            document.mark_feature_dirty(id);
        }
    }

    fn invalidate_all(&self, document: &mut Document) {
        let all: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND)
            .map(|(id, _)| *id)
            .collect();
        for id in all {
            document.mark_feature_dirty(id);
        }
    }

    /// Every element, constructed element, dimension and tolerance as one
    /// table, the tally above it, the scans they are on pictured.
    fn semantic_pmi(&self, document: &Document, body: BodyId) -> core_document::SemanticPmi {
        pmi::of_body(document, body)
    }

    fn report(&self, document: &Document) -> Option<core_document::BenchReport> {
        let report = report_of(
            document,
            None,
            &self.tab.tolerance_readings,
            self.kernel.get().copied(),
            true,
        );
        let rows = report::rows(&report);
        if rows.is_empty() {
            return None;
        }
        let mut bodies: Vec<BodyId> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == element::KIND)
            .filter_map(|(_, n)| n.body)
            .collect();
        bodies.sort();
        bodies.dedup();
        Some(core_document::BenchReport {
            title: "Inspection".to_string(),
            summary: vec![format!(
                "{} passed, {} failed",
                report["passed"], report["failed"]
            )],
            tables: vec![core_document::ReportTable {
                heading: "Elements, dimensions and tolerances".to_string(),
                columns: [
                    "Section",
                    "Name",
                    "What",
                    "Value",
                    "Target",
                    "Deviation",
                    "Result",
                    "Note",
                ]
                .map(String::from)
                .to_vec(),
                rows: rows.into_iter().map(|r| r.to_vec()).collect(),
            }],
            bodies,
        })
    }

    fn feature_solid(
        &self,
        document: &Document,
        id: core_document::FeatureId,
    ) -> Option<kernel_api::MeshOperand> {
        let e = element_at(document, id)?;
        e.result.solid(document.body_placement(e.scan).rows())
    }

    /// A section offers its first cut's plane, a fitted or constructed
    /// plane its own, where the scan stands.
    fn feature_plane(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        node: &FeatureNode,
    ) -> Option<kernel_api::ProfilePlane> {
        let (scan, origin, normal) = if node.workbench_id.as_str() == section::KIND {
            let s = document.feature_as::<section::Section>(id)?;
            let n = DVec3::from(s.normal);
            vec::unit(n)?;
            let shift = n * (s.offset / n.length());
            (s.scan, (DVec3::from(s.origin) + shift).to_array(), s.normal)
        } else {
            let e = element_at(document, id)?;
            match e.result.shape {
                kernel_api::FittedShape::Plane { origin, normal } => (e.scan, origin, normal),
                _ => return None,
            }
        };
        let rows = document.body_placement(scan).rows();
        let place = |p: [f64; 3], w: f64| -> [f64; 3] {
            std::array::from_fn(|r| {
                rows[r][0] * p[0] + rows[r][1] * p[1] + rows[r][2] * p[2] + w * rows[r][3]
            })
        };
        // A plane with no normal offers nothing to sketch on.
        let n = vec::unit(normal.into())?;
        let (x, y) = vec::basis(n)?;
        Some(kernel_api::ProfilePlane {
            origin: place(origin, 1.0),
            normal: place(n.to_array(), 0.0),
            x_axis: place(x.to_array(), 0.0),
            y_axis: place(y.to_array(), 0.0),
        })
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.handle_input(event, active_tool, ctx)
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        if let Some(kernel) = ctx.kernel {
            let _ = self.kernel.set(kernel);
        }
        commands::run(self, id, args, ctx)
    }
}

#[cfg(test)]
mod gdt_tests;

#[cfg(test)]
mod best_fit_tests;

#[cfg(test)]
mod tests;
