//! Design workbench: feature-based solid modeling.
//!
//! The workbench edits the document's feature tree; the app shell watches
//! for dirty part features and drives the kernel rebuild (see `build.rs`).

// Without the egui feature the crate is the headless subset its dependents'
// tests and the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code, unused_imports))]

/// The feature kind Design's features are stored under.
pub const KIND: &str = "wb.design";

mod borrow;
mod build;
mod centre;
mod clipboard;
mod commands;
#[cfg(feature = "egui")]
mod datum_panel;
mod datum_refs;
#[cfg(feature = "egui")]
mod editors;
mod face_snap;
mod failing;
mod feature;
mod features;
#[cfg(feature = "egui")]
mod form;
mod generators;
mod handles;
mod hole_tables;
mod input;
mod instances;
mod making;
mod menus;
mod params;
mod profile_repair;
mod read_scan;
mod recognize;
mod references;
mod region_pick;
mod registration;
pub mod scan_deviation;
mod section;
mod separate;
mod size_limit;
mod snap;
mod split;
#[cfg(feature = "egui")]
mod task;
#[cfg(test)]
mod test_support;
mod text_feature;
mod tools;
mod viewport_out;

pub use borrow::freeze;
pub use build::{
    BuildError, BuildPlan, NutPocket, body_build_ops, delete_feature, design_feature_ids,
    design_features_of_body, hole_diameter, invalidate_body, mark_all_design_features_dirty,
    nut_pocket, pending_body_rebuilds, rebuild_jobs, sketch_plane_description, sketches_of_body,
};
pub use feature::{
    Attached, BaseAxis, BorrowOptions, BorrowSource, BorrowedRef, ChamferMode, DesignFeature,
    DrillPoint, EdgePick, EdgeSel, ExtrudeDirection, ExtrudeExtras, ExtrudeMode, FacePick,
    FrozenBorrow, FrozenEdge, FrozenFace, HelixMode, HoleCut, HoleFit, HoleStart, LoftSection,
    MirrorPlane, NutSide, NutTrap, PathSpacing, PatternAxis, PatternPath, PipeCorner,
    PipeOrientation, PlaneTarget, RevolveAxis, RevolveMode, ScaleCentre, SketchAxis, ThreadSpec,
    TransformStep, primitive_icon, primitive_preset,
};
pub use hole_tables::{
    CUT_PROFILES_FILE, CutProfile, NutSize, NutStandard, ScrewSeat, ThreadSize, ThreadStandard,
    parse_cut_profiles, user_cut_profiles,
};
pub use references::retarget_feature_sketch;

#[cfg(test)]
use core_document::HostRequest;
use core_document::{
    BodyId, Document, FeatureId, FeatureInfo, InputResult, MenuItem, MenuScope, TaskInfo,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext, base_tool_id,
};
use features::datum_icon;
use making::{Given, ToolMade, take_base};
use registration::PRIMITIVE_SHAPES;
#[cfg(all(test, feature = "egui"))]
use registration::TOOL_KEYS;
#[cfg(test)]
use viewport_out::datum_mesh;

const MOVE_TO_BODY: &str = "design.move_to_body";
const DUPLICATE: &str = "design.duplicate";
const SEPARATE: &str = "design.separate";
const SECTION: &str = "design.section";
const MAP_SKETCH: &str = "design.map_sketch";
const SPLIT: &str = "design.split";

/// The switches on the Design Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DesignOptions {
    /// The preview rebuilds on every field change in the task panel; off,
    /// it rebuilds when the task is accepted.
    pub update_while_editing: bool,
    /// A sketch a new feature consumes is hidden.
    pub hide_used_sketches: bool,
    /// A new feature that fuses or cuts merges the coplanar faces it leaves.
    pub refine_result: bool,
    /// The step a view handle's drag moves in, mm.
    pub handle_step: f32,
    /// The step while Shift is held, mm.
    pub handle_fine_step: f32,
    /// The step an angle handle's drag turns in, degrees.
    pub handle_angle_step: f32,
    /// The angle step while Shift is held, degrees.
    pub handle_fine_angle_step: f32,
}

impl Default for DesignOptions {
    fn default() -> Self {
        Self {
            update_while_editing: true,
            hide_used_sketches: true,
            refine_result: true,
            handle_step: 1.0,
            handle_fine_step: 0.1,
            handle_angle_step: 5.0,
            handle_fine_angle_step: 1.0,
        }
    }
}

#[cfg(feature = "egui")]
impl DesignOptions {
    /// The Preferences page, declared.
    fn settings(&self) -> Vec<bench_api::Widget> {
        use bench_api::{Check, Quantity, SettingsGroup, Widget};
        let switch = |id: &str, text: &str, on: bool, hint: &str| Widget::Hinted {
            hint: hint.into(),
            widget: Box::new(
                Check {
                    id: id.into(),
                    text: text.into(),
                    on,
                    ..Default::default()
                }
                .into(),
            ),
        };
        let step = |id: &str, label: &str, value: f32, (unit, max): (&str, f64), hint: &str| {
            Widget::Hinted {
                hint: hint.into(),
                widget: Box::new(
                    Quantity {
                        id: id.into(),
                        label: label.into(),
                        value: f64::from(value),
                        unit: Some(unit.into()),
                        decimals: 2,
                        min: Some(0.01),
                        max: Some(max),
                        width: Some(120.0),
                        ..Default::default()
                    }
                    .into(),
                ),
            }
        };
        vec![Widget::Settings(SettingsGroup {
            title: "Feature defaults".into(),
            children: vec![
                switch(
                    "refine_result",
                    "Refine result",
                    self.refine_result,
                    "New features merge the coplanar faces their fuse or cut leaves",
                ),
                switch(
                    "update_while_editing",
                    "Update view while editing",
                    self.update_while_editing,
                    "Rebuild the preview on every field change; off, on OK",
                ),
                switch(
                    "hide_used_sketches",
                    "Hide the sketch after a feature uses it",
                    self.hide_used_sketches,
                    "Keep used sketches out of the viewport",
                ),
                step(
                    "handle_step",
                    "Handle step",
                    self.handle_step,
                    ("mm", 100.0),
                    "A view handle's drag moves the number in these steps",
                ),
                step(
                    "handle_fine_step",
                    "Fine handle step",
                    self.handle_fine_step,
                    ("mm", 100.0),
                    "The step while Shift is held",
                ),
                step(
                    "handle_angle_step",
                    "Angle handle step",
                    self.handle_angle_step,
                    ("°", 90.0),
                    "An angle handle's drag turns the number in these steps",
                ),
                step(
                    "handle_fine_angle_step",
                    "Fine angle handle step",
                    self.handle_fine_angle_step,
                    ("°", 90.0),
                    "The angle step while Shift is held",
                ),
            ],
        })]
    }

    /// What a change on the Preferences page sets.
    fn settings_event(&mut self, event: &bench_api::PanelEvent) {
        use bench_api::PanelEvent;
        match event {
            PanelEvent::Toggle { id, on } => match id.as_str() {
                "refine_result" => self.refine_result = *on,
                "update_while_editing" => self.update_while_editing = *on,
                "hide_used_sketches" => self.hide_used_sketches = *on,
                _ => {}
            },
            PanelEvent::Number { id, value } => {
                let value = *value as f32;
                match id.as_str() {
                    "handle_step" => self.handle_step = value,
                    "handle_fine_step" => self.handle_fine_step = value,
                    "handle_angle_step" => self.handle_angle_step = value,
                    "handle_fine_angle_step" => self.handle_fine_angle_step = value,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// What the bench holds for the document on screen, moved with its tab:
/// everything but its settings.
#[derive(Default)]
struct Tab {
    /// The feature open in the task panel.
    #[cfg(feature = "egui")]
    task: Option<task::TaskState>,
    /// A feature a tool just created: the task that opens for it deletes it
    /// on Cancel, and records it on OK as the command that makes it.
    pending_task_from_tool: Option<ToolMade>,
    /// A feature the user asked to edit (a double click in the tree), for
    /// the task panel to open on.
    edit_request: Option<FeatureId>,
    /// The centre line tool, while it is out.
    centre: Option<centre::CentreTask>,
    /// Features Edit › Copy or Cut took.
    clipboard: Option<clipboard::Clipboard>,
    /// The drag handle held, as it was taken and at the number it sets.
    held: Option<handles::Handle>,
    /// The held handle's feature as the press found it, which a cancel
    /// writes back.
    held_from: Option<serde_json::Value>,
    /// The occurrence of the open pattern whose dot is under the pointer.
    hovered_occurrence: Option<u32>,
    /// The held handle has been dragged: a release then keeps the number
    /// rather than turning the feature round.
    held_moved: bool,
    /// The body whose datums the view draws while the bench is active,
    /// which are the datums a click can take.
    datums_drawn: Option<BodyId>,
    /// The scans are painted by their distance to the body modelled.
    scan_deviation: bool,
    /// The reading tool armed (Read round or Read thread): a click on a
    /// scan reads what is there.
    reading: Option<&'static str>,
    /// Each scan's distances to the body, by what they were measured from.
    deviations: core_document::jobs::Keyed<BodyId, std::sync::Arc<[f32]>>,
    /// The failing edge whose row in the task panel is under the pointer,
    /// by its place in the feature's list.
    hovered_failing: Option<usize>,
    /// Why freezing a borrow failed, by the borrow, shown in its task
    /// until the next try.
    #[cfg(feature = "egui")]
    freeze_errors: std::collections::HashMap<FeatureId, String>,
    /// A value typed for a handle taken: the handle, and the text so far.
    /// It stays open past the release until Enter, Escape or a click.
    typed: Option<(handles::Handle, String)>,
    /// The face the held handle's length snapped to, lit.
    snapped: Option<face_snap::Snap>,
    /// The lengths the held handle's drag has set, its feature's own end
    /// standing at one of them.
    dragged: Vec<f32>,
}

/// Design workbench: feature-based solid modeling.
#[derive(Default)]
pub struct DesignWorkbench {
    /// The Preferences page's switches.
    pub options: DesignOptions,
    /// What the bench holds for the document on screen, moved with its
    /// tab.
    tab: Tab,
}

impl DesignWorkbench {
    /// The feature whose task is open.
    fn task_feature(&self) -> Option<FeatureId> {
        #[cfg(feature = "egui")]
        {
            self.tab.task.as_ref().map(|t| t.feature)
        }
        #[cfg(not(feature = "egui"))]
        {
            None
        }
    }
}

impl Workbench for DesignWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            crate::KIND,
            "Design",
            "Feature-based solid modeling workbench.",
        )
        .icon("workbench-part-design")
        .feature_kinds([crate::KIND, "core.datum"])
        .radial([
            "design.pad",
            "design.pocket",
            "design.hole",
            "design.fillet",
            "design.chamfer",
            "design.new_sketch",
            "design.revolve",
            "design.mirror",
        ])
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<core_document::RebuildJob> {
        build::rebuild_jobs(document)
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        build::invalidate_body(document, body);
    }

    fn invalidate_all(&self, document: &mut Document) {
        build::mark_all_design_features_dirty(document);
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.task_feature()
    }

    fn references(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Vec<core_document::FeatureReference> {
        references::references(document, id, node)
    }

    fn set_reference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: FeatureId,
        key: &str,
        to: core_document::ReferenceChoice,
    ) -> Result<(), String> {
        references::set_reference(ctx, id, key, to)
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        #[cfg(feature = "egui")]
        if task::task_kind(ctx, id).is_some() {
            self.tab.edit_request = Some(id);
        }
        #[cfg(not(feature = "egui"))]
        let _ = (ctx, id);
    }

    fn feature_info(&self, node: &core_document::FeatureNode) -> FeatureInfo {
        self.info_of(node)
    }

    /// A feature swept, cut or drilled from a sketch consumes that
    /// sketch; every other one (a dress-up, a pattern, a mirror, a
    /// boolean, a loft of several sections) works on the body and sits at
    /// its top level. A datum consumes its one reference.
    fn consumed_feature(
        &self,
        document: &Document,
        node: &core_document::FeatureNode,
    ) -> Option<FeatureId> {
        if node.workbench_id.as_str() == "core.datum" {
            return core_document::workbench::sole_dependency(document, node);
        }
        DesignFeature::from_json(&node.data).ok()?.sketch()
    }

    /// A hole with a cosmetic thread threads each of its bores; a mirror
    /// or a pattern each copy of them it keeps.
    fn cosmetic_threads(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Vec<core_document::ThreadCallout> {
        self.threads_of(document, id, node)
    }

    /// A borrow draws what it lends where its body sits, in the palette's
    /// external colour: the sketch's curves, or the outlines of the faces
    /// and edges.
    fn passive_geometry(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
    ) -> Option<core_document::PassiveGeometry> {
        self.passive_of(document, id, node)
    }

    /// A datum the view draws, measured to the nearest of its lines on
    /// screen.
    fn pick_feature(
        &self,
        document: &Document,
        id: FeatureId,
        node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
    ) -> Option<f32> {
        self.pick_of(document, id, node, pick)
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        self.register_tools(context);
    }

    /// A feature row offers Duplicate, and a move into each other body
    /// built here (not one read from a file); a sketch's row offers each
    /// flat face its body borrows to map it onto.
    fn menu_items(&self, scope: &MenuScope, document: &Document) -> Vec<MenuItem> {
        self.menu_entries(scope, document)
    }

    fn on_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        self.menu_command(id, scope, ctx)
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        match base_tool_id(tool_id) {
            "design.scan_deviation" => self.tab.scan_deviation,
            id @ ("design.read_hole" | "design.read_thread" | "design.snap_to_scan") => {
                self.tab.reading == Some(id)
            }
            _ => false,
        }
    }

    fn busy(&self) -> bool {
        self.tab.deviations.busy()
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            meshes: self.overlay_meshes(ctx, ctx.active_document_object),
            overlays: self.screen_overlays(ctx, ctx.active_document_object),
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            hud: self.viewport_hud(ctx),
            scalar_maps: self.scalar_maps(ctx),
            status: self.status_items(ctx),
            ..Default::default()
        }
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        if generators::is_command(id) {
            return generators::command(id, args, ctx);
        }
        commands::run(self, id, args, ctx)
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Design workbench activated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.take_input(event, active_tool, ctx)
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        if self.tab.centre.is_some() {
            return Some(TaskInfo {
                title: "Centre line".to_string(),
                icon: centre::ICON,
                confirmable: false,
                stepwise: false,
                ..Default::default()
            });
        }
        #[cfg(feature = "egui")]
        {
            self.task_info(ctx)
        }
        #[cfg(not(feature = "egui"))]
        {
            let _ = ctx;
            None
        }
    }

    fn cancel_pointer_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.put_back_handle(ctx);
    }

    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.let_go_of_handle();
        self.tab.centre = None;
        #[cfg(feature = "egui")]
        {
            self.tab.task = None;
        }
    }

    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {
        self.tab.centre = None;
        self.tab.datums_drawn = None;
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        self.tab.datums_drawn = Self::target_body(ctx);
        if self.tab.centre.is_some() {
            self.take_centre_pick(ctx);
        }
    }

    /// While a view handle is held: what it sets, its steps and the key
    /// for the fine ones.
    /// Digits typed while a handle is held set its number.
    fn takes_numeric_input(&self) -> bool {
        self.tab.held.is_some() || self.tab.typed.is_some()
    }

    /// Once typing has begun, letters too: a formula.
    fn takes_text_input(&self) -> bool {
        self.tab.typed.is_some()
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        self.tool_enabled(tool_id, ctx)
    }

    fn has_settings(&self) -> bool {
        true
    }

    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        build::delete_feature(ctx.document, id)
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.options).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        if let Ok(options) = serde_json::from_value(value.clone()) {
            self.options = options;
        }
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        Some(&mut self.tab)
    }

    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        if node.workbench_id.as_str() == "core.datum" {
            params::datum_parameters()
        } else {
            params::feature_parameters(node)
        }
    }

    /// A fillet's or chamfer's sizes drag up to the largest that builds.
    fn parameter_drag_range(
        &self,
        document: &Document,
        feature: FeatureId,
        parameter: &core_document::Parameter,
    ) -> (Option<f64>, Option<f64>) {
        let most = size_limit::drag_most(document, feature, &parameter.pointer);
        (None, most.map(f64::from))
    }

    /// A primitive attached by a mode, or a feature starting on a face,
    /// takes what the last build found of what it stands on.
    fn derive_on_solid(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        probed: &core_document::rebuild::ProbedReferences,
    ) -> bool {
        self.follow_solid(node, values, probed)
    }

    /// A datum made from a sketch's points or lines takes where they stand
    /// now; so does a primitive attached by a mode, which follows the
    /// datums it stands on too.
    fn derive(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        _document: &core_document::Document,
    ) -> bool {
        self.derive_values(node, values, values_of, _document)
    }

    fn property_hints(&self) -> core_document::PropertyHints {
        core_document::PropertyHints {
            length_keys: vec![
                "length",
                "length2",
                "depth",
                "depth2",
                "radius",
                "size",
                "size2",
                "value",
                "diameter",
                "pitch",
                "height",
                "up_to_offset",
                "width",
                "circumradius",
                "radius1",
                "radius2",
                "radius3",
            ],
            reference_keys: vec![
                "sketch",
                "profile",
                "spine",
                "sections",
                "originals",
                "tool_body",
            ],
        }
    }
}

#[cfg(feature = "egui")]
impl bench_panel::BenchUi for DesignWorkbench {
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        if self.tab.centre.is_some() {
            return self.centre_panel(ui, request);
        }
        let outcome = self.draw_task_panel(ui, ctx, request);
        // A handle held as its task closes is let go, the task having
        // settled its number.
        if !matches!(outcome, core_document::TaskOutcome::Open) {
            self.let_go_of_handle();
        }
        outcome
    }

    /// The Design preferences page.
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) {
        let widgets = self.options.settings();
        let out = bench_panel::show_filtered(
            ui,
            egui::Id::new("design_settings"),
            &widgets,
            None,
            filter,
        );
        for event in &out.events {
            self.options.settings_event(event);
        }
    }

    /// Under the tree the bench only orients a new user; features are
    /// edited in the task panel and managed from the tree's menu.
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        if Self::target_body(ctx).is_some() {
            return;
        }
        ui.label(
            egui::RichText::new("Create a body, then a sketch on it, then Pad the sketch.")
                .font(ui_kit::sans(ui_kit::tokens::FONT_SM))
                .color(ui_kit::tokens::TEXT3),
        );
        ui.label(
            egui::RichText::new("Select a body or sketch in the tree to see its features.")
                .font(ui_kit::sans(ui_kit::tokens::FONT_SM))
                .color(ui_kit::tokens::TEXT3),
        );
    }
}

#[cfg(test)]
mod body_tool_tests;

#[cfg(all(test, feature = "egui"))]
mod icon_coverage_tests;

#[cfg(test)]
mod consumed_tests;
