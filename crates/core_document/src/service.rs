//! Workbench registry used by the application shell.

use std::collections::HashMap;

use crate::Document;
use crate::feature::FeatureId;
use crate::feature::{BodyId, FeatureNode};
use crate::rebuild::RebuildJob;
use crate::workbench::{
    FeatureInfo, MenuItem, MenuScope, PassiveGeometry, PropertyHints, ToolDescriptor, ViewportPick,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchId,
};
use crate::{DocumentError, DocumentResult};

/// Central registry tracking workbenches and their declared capabilities.
#[derive(Default)]
pub struct DocumentService {
    workbenches: HashMap<String, WorkbenchEntry>,
    /// Bench ids in registration order: the order the Preferences rail
    /// lists them in, and the order the first non-modal bench is found in.
    order: Vec<WorkbenchId>,
    /// Feature kind → the bench that claimed it.
    owners: HashMap<String, WorkbenchId>,
}

impl std::fmt::Debug for DocumentService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentService")
            .field("workbenches", &self.order)
            .finish()
    }
}

struct WorkbenchEntry {
    descriptor: WorkbenchDescriptor,
    workbench: Box<dyn Workbench>,
    context: WorkbenchContext,
}

impl DocumentService {
    pub fn register_workbench(&mut self, workbench: Box<dyn Workbench>) -> DocumentResult<()> {
        let descriptor = workbench.descriptor();
        if self.workbenches.contains_key(descriptor.id.as_str()) {
            return Err(DocumentError::WorkbenchExists(
                descriptor.id.as_str().to_owned(),
            ));
        }
        for kind in &descriptor.feature_kinds {
            if let Some(by) = self.owners.get(kind.as_str()) {
                return Err(DocumentError::FeatureKindClaimed {
                    kind: kind.as_str().to_owned(),
                    by: by.as_str().to_owned(),
                });
            }
        }

        let mut context = WorkbenchContext::default();
        workbench.configure(&mut context);
        for command in context.commands() {
            if let Some((by, _)) = self.command(&command.id) {
                return Err(DocumentError::CommandClaimed {
                    id: command.id.clone(),
                    by: by.as_str().to_owned(),
                });
            }
        }

        for kind in &descriptor.feature_kinds {
            self.owners
                .insert(kind.as_str().to_owned(), descriptor.id.clone());
        }
        self.order.push(descriptor.id.clone());
        self.workbenches.insert(
            descriptor.id.as_str().to_owned(),
            WorkbenchEntry {
                descriptor,
                workbench,
                context,
            },
        );

        Ok(())
    }

    /// Take workbench `id` out of the registry, with its feature kinds,
    /// tools and commands: a package being removed or replaced while the
    /// app runs. Features of its kinds stay in documents, unowned until a
    /// bench claims them again. The caller moves any session off it first.
    pub fn unregister_workbench(&mut self, id: &WorkbenchId) -> Option<Box<dyn Workbench>> {
        let entry = self.workbenches.remove(id.as_str())?;
        self.order.retain(|o| o != id);
        self.owners.retain(|_, owner| owner != id);
        Some(entry.workbench)
    }

    pub fn workbench_descriptors(&self) -> impl Iterator<Item = &WorkbenchDescriptor> {
        self.workbenches.values().map(|entry| &entry.descriptor)
    }

    /// Bench ids in registration order.
    pub fn ids(&self) -> &[WorkbenchId] {
        &self.order
    }

    pub fn descriptor(&self, id: &WorkbenchId) -> Option<&WorkbenchDescriptor> {
        self.workbenches.get(id.as_str()).map(|e| &e.descriptor)
    }

    /// The bench a new document lands in: the first registered bench that
    /// is not an edit-session bench.
    pub fn landing_workbench(&self) -> Option<WorkbenchId> {
        self.order
            .iter()
            .find(|id| self.descriptor(id).is_some_and(|d| !d.modal))
            .cloned()
    }

    /// Whether entering `id` is an edit-session excursion to return from.
    pub fn is_modal(&self, id: &WorkbenchId) -> bool {
        self.descriptor(id).is_some_and(|d| d.modal)
    }

    /// The bench that claimed feature kind `kind`.
    pub fn owner_id_of(&self, kind: &WorkbenchId) -> Option<&WorkbenchId> {
        self.owners.get(kind.as_str())
    }

    pub fn owner_of(&self, kind: &WorkbenchId) -> Option<&dyn Workbench> {
        let id = self.owner_id_of(kind)?;
        self.workbenches
            .get(id.as_str())
            .map(|e| e.workbench.as_ref())
    }

    /// How `node` presents, asked of the bench that claimed its kind;
    /// `None` when no bench did.
    pub fn feature_info(&self, node: &FeatureNode) -> Option<FeatureInfo> {
        if node.workbench_id.as_str() == crate::configurations::CONFIGURATIONS_KIND {
            return Some(FeatureInfo {
                icon: "expression",
                kind_label: "Configurations".to_string(),
                family_label: "Variables".to_string(),
                builds_solid: false,
            });
        }
        if node.workbench_id.as_str() == crate::variables::VARIABLES_KIND {
            return Some(FeatureInfo {
                icon: "expression",
                kind_label: "Variable set".to_string(),
                family_label: "Variables".to_string(),
                builds_solid: false,
            });
        }
        self.owner_of(&node.workbench_id)
            .map(|wb| wb.feature_info(node))
    }

    fn benches(&self) -> impl Iterator<Item = &dyn Workbench> {
        self.order
            .iter()
            .filter_map(|id| self.workbenches.get(id.as_str()))
            .map(|e| e.workbench.as_ref())
    }

    /// Whether any bench has work running away from the window.
    pub fn any_busy(&self) -> bool {
        self.benches().any(|wb| wb.busy())
    }

    /// Every bench's settings, keyed by bench id.
    pub fn collect_settings(&self) -> HashMap<String, serde_json::Value> {
        self.order
            .iter()
            .filter_map(|id| {
                let value = self
                    .workbenches
                    .get(id.as_str())?
                    .workbench
                    .settings_json()?;
                Some((id.as_str().to_owned(), value))
            })
            .collect()
    }

    /// Give every bench its settings back.
    pub fn apply_settings(&mut self, settings: &HashMap<String, serde_json::Value>) {
        for (id, value) in settings {
            if let Some(entry) = self.workbenches.get_mut(id.as_str()) {
                entry.workbench.apply_settings_json(value);
            }
        }
    }

    /// Every bench's editing state for the document on screen, keyed by
    /// bench id; the benches are left as if no document were open.
    pub fn suspend_sessions(&mut self) -> HashMap<String, Box<dyn std::any::Any + Send>> {
        let mut states = HashMap::new();
        for id in &self.order {
            if let Some(entry) = self.workbenches.get_mut(id.as_str())
                && let Some(state) = entry.workbench.suspend_session()
            {
                states.insert(id.as_str().to_owned(), state);
            }
        }
        states
    }

    /// Give every bench its state back for the tab coming on screen; a
    /// bench with nothing stored starts from nothing.
    pub fn resume_sessions(&mut self, mut states: HashMap<String, Box<dyn std::any::Any + Send>>) {
        for id in &self.order {
            if let Some(entry) = self.workbenches.get_mut(id.as_str()) {
                entry.workbench.resume_session(states.remove(id.as_str()));
            }
        }
    }

    /// Every bench's rebuild jobs, in registration order.
    /// The numeric properties of `node`, from the bench that claimed its
    /// kind.
    pub fn parameters(&self, node: &FeatureNode) -> Vec<crate::evaluate::Parameter> {
        self.owner_of(&node.workbench_id)
            .map(|wb| wb.parameters(node))
            .unwrap_or_default()
    }

    /// Work out the document's formulas when it changed since they last
    /// were; features whose values moved are marked for rebuilding.
    pub fn evaluate(&self, document: &mut Document) {
        if !document.needs_evaluation() {
            return;
        }
        let mut evaluation =
            crate::evaluate::evaluate_document(document, &|node| self.parameters(node));
        let ids: Vec<FeatureId> = evaluation.data.keys().copied().collect();
        for id in ids {
            let data = evaluation.data.get_mut(&id).expect("listed");
            // The same values as last time settle the same way.
            if let Some(settled) = document.settled_values(id, data) {
                *data = settled.clone();
                evaluation.unsettled.insert(id, data.clone());
                continue;
            }
            let unsettled = data.clone();
            if let Some(node) = document.get_feature_meta(id)
                && let Some(owner) = self.owner_of(&node.workbench_id)
            {
                owner.settle(node, data);
            }
            evaluation.unsettled.insert(id, unsettled);
        }
        document.apply_evaluation(evaluation);
    }

    /// The parameter of `feature` that formulas call `name`, or whose key
    /// is `name`.
    pub fn parameter_named(
        &self,
        document: &Document,
        feature: FeatureId,
        name: &str,
    ) -> Result<crate::evaluate::Parameter, String> {
        let node = document
            .get_feature_meta(feature)
            .ok_or("no such feature")?;
        let params = self.parameters(node);
        params
            .iter()
            .find(|p| p.name.as_deref() == Some(name) || p.key == name)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<String> = params
                    .iter()
                    .map(|p| p.name.clone().unwrap_or_else(|| p.key.clone()))
                    .collect();
                format!(
                    "{} has no number {name}; it has {}",
                    node.name,
                    names.join(", ")
                )
            })
    }

    /// Set `parameter` of `feature` to `value` (millimetres or degrees):
    /// any formula on it goes, the number goes into the feature's data,
    /// settled by its bench (a sketch solves), and what depends on it is
    /// marked, as are the benches' follow-ups (`values_moved`).
    pub fn set_parameter_value(
        &self,
        document: &mut Document,
        feature: FeatureId,
        parameter: &crate::evaluate::Parameter,
        value: f64,
    ) -> Result<(), String> {
        let node = document
            .get_feature_meta(feature)
            .cloned()
            .ok_or("no such feature")?;
        let mut data = node.data.clone();
        let slot = data
            .pointer_mut(&parameter.pointer)
            .ok_or_else(|| format!("{} has no {} now", node.name, parameter.label))?;
        let stored = value * parameter.scale;
        *slot = if parameter.integer {
            serde_json::json!(stored.round() as i64)
        } else {
            serde_json::json!(stored)
        };
        if let Some(owner) = self.owner_of(&node.workbench_id) {
            owner.settle(&node, &mut data);
        }
        document
            .set_feature_formula(feature, parameter.key.clone(), None)
            .map_err(|e| e.to_string())?;
        document
            .update_feature_data(feature, data)
            .map_err(|e| e.to_string())?;
        document.mark_feature_dirty(feature);
        document.note_value_moved(feature);
        Ok(())
    }

    /// What `text` comes to in `document`: for a field holding `want`, or
    /// as it is.
    pub fn evaluate_formula(
        &self,
        document: &Document,
        text: &str,
        want: Option<crate::expr::Dim>,
    ) -> Result<crate::expr::Quantity, String> {
        crate::evaluate::evaluate_formula(document, &|node| self.parameters(node), text, want)
    }

    /// Every bench's rebuilds, after the formulas are worked out. A bench
    /// that panics while planning costs only its own rebuilds: its dirty
    /// features are settled, carrying the panic as their error, so it does
    /// not panic again every frame and the app keeps running.
    pub fn rebuild_jobs(&self, document: &mut Document) -> Vec<RebuildJob> {
        self.evaluate(document);
        let mut jobs = Vec::new();
        for id in &self.order {
            let Some(entry) = self.workbenches.get(id.as_str()) else {
                continue;
            };
            let planned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                entry.workbench.rebuild_jobs(document)
            }));
            match planned {
                Ok(planned) => jobs.extend(planned),
                Err(panic) => {
                    let why = panic
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "an unknown failure".into());
                    let message = format!(
                        "{} failed planning this rebuild: {why}",
                        entry.descriptor.label
                    );
                    tracing::error!(target: "printcad.bench", "{message}");
                    let kinds = &entry.descriptor.feature_kinds;
                    let dirty: Vec<FeatureId> = document
                        .feature_tree()
                        .all_nodes()
                        .filter(|(_, n)| n.dirty && kinds.contains(&n.workbench_id))
                        .map(|(id, _)| *id)
                        .collect();
                    for feature in dirty {
                        document.clear_feature_dirty(feature);
                        document.set_feature_error(feature, Some(message.clone()));
                    }
                }
            }
        }
        jobs
    }

    pub fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        for wb in self.benches() {
            wb.invalidate_body(document, body);
        }
    }

    pub fn invalidate_all(&self, document: &mut Document) {
        for wb in self.benches() {
            wb.invalidate_all(document);
        }
    }

    /// The 3D presence of every visible feature not under edit, asked of
    /// the bench that claimed its kind.
    pub fn passive_geometries(
        &self,
        document: &Document,
        editing: Option<FeatureId>,
    ) -> Vec<(FeatureId, PassiveGeometry)> {
        document
            .feature_tree()
            .all_nodes()
            .filter(|(id, node)| node.visible && Some(**id) != editing)
            .filter_map(|(id, node)| {
                let mut geometry = self
                    .owner_of(&node.workbench_id)?
                    .passive_geometry(document, *id, node)?;
                // A bench draws in its body's frame; the scene has it where
                // the body sits.
                let placement = node
                    .body
                    .map(|body| document.body_placement(body))
                    .unwrap_or_default();
                if !placement.is_identity() {
                    geometry.mesh = placement.mesh(&geometry.mesh);
                    geometry.revision = mix_placement(geometry.revision, &placement);
                }
                Some((*id, geometry))
            })
            .collect()
    }

    /// The visible feature nearest the cursor within `tolerance_px`, asked
    /// of the bench that claimed each feature's kind.
    pub fn pick_feature(
        &self,
        document: &Document,
        pick: &ViewportPick,
        tolerance_px: f32,
    ) -> Option<FeatureId> {
        document
            .feature_tree()
            .all_nodes()
            .filter(|(_, node)| node.visible)
            .filter_map(|(id, node)| {
                // The bench measures in its body's frame: the view it is
                // given places that frame where the body sits.
                let placement = node
                    .body
                    .map(|body| document.body_placement(body))
                    .unwrap_or_default();
                let placed_pick;
                let pick = if placement.is_identity() {
                    pick
                } else {
                    let view_proj = glam::Mat4::from_cols_array_2d(&pick.view_proj)
                        * glam::Mat4::from_cols_array_2d(&placement.matrix());
                    placed_pick = ViewportPick {
                        view_proj: view_proj.to_cols_array_2d(),
                        ..*pick
                    };
                    &placed_pick
                };
                let distance = self
                    .owner_of(&node.workbench_id)?
                    .pick_feature(document, *id, node, pick)?;
                (distance <= tolerance_px).then_some((*id, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Every bench's entries for a contextual menu, in registration order,
    /// each with the bench that offers it.
    pub fn menu_items(
        &self,
        scope: &MenuScope,
        document: &Document,
    ) -> Vec<(WorkbenchId, MenuItem)> {
        self.order
            .iter()
            .filter_map(|id| self.workbenches.get(id.as_str()).map(|e| (id, e)))
            .flat_map(|(id, entry)| {
                entry
                    .workbench
                    .menu_items(scope, document)
                    .into_iter()
                    .map(move |item| (id.clone(), item))
            })
            .collect()
    }

    /// Every bench's property hints, merged.
    pub fn property_hints(&self) -> PropertyHints {
        let mut hints = PropertyHints::default();
        for wb in self.benches() {
            hints.merge(wb.property_hints());
        }
        hints
    }

    pub fn tools_for(&self, id: &WorkbenchId) -> DocumentResult<&[ToolDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.tools())
    }

    /// Every workbench's commands, in registration order, each with the
    /// workbench that runs it.
    pub fn commands(&self) -> Vec<(WorkbenchId, &crate::CommandSpec)> {
        let mut out = Vec::new();
        for id in self.ids() {
            if let Some(entry) = self.workbenches.get(id.as_str()) {
                out.extend(entry.context.commands().iter().map(|c| (id.clone(), c)));
            }
        }
        out
    }

    /// The command `id` and the workbench that runs it.
    pub fn command(&self, id: &str) -> Option<(WorkbenchId, &crate::CommandSpec)> {
        self.commands().into_iter().find(|(_, c)| c.id == id)
    }

    /// Tell every workbench the keys in effect, by tool or action id.
    pub fn notify_shortcuts(
        &mut self,
        keys: &std::collections::HashMap<String, Vec<crate::shortcut::Chord>>,
    ) {
        for entry in self.workbenches.values_mut() {
            entry.workbench.shortcuts_changed(keys);
        }
    }

    /// The keyboard actions a workbench registered beside its tools.
    pub fn actions_for(
        &self,
        id: &WorkbenchId,
    ) -> DocumentResult<&[crate::shortcut::ActionDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.actions())
    }

    pub fn workbench(&self, id: &WorkbenchId) -> DocumentResult<&dyn Workbench> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.workbench.as_ref())
    }

    pub fn workbench_mut(&mut self, id: &WorkbenchId) -> DocumentResult<&mut Box<dyn Workbench>> {
        let entry = self
            .workbenches
            .get_mut(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(&mut entry.workbench)
    }
}

/// A passive geometry revision that also changes when its body moves.
fn mix_placement(revision: u64, placement: &crate::BodyPlacement) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    revision.hash(&mut h);
    for v in placement.translation.iter().chain(&placement.rotation) {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}
