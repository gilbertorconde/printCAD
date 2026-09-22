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
        self.owner_of(&node.workbench_id)
            .map(|wb| wb.feature_info(node))
    }

    fn benches(&self) -> impl Iterator<Item = &dyn Workbench> {
        self.order
            .iter()
            .filter_map(|id| self.workbenches.get(id.as_str()))
            .map(|e| e.workbench.as_ref())
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
    pub fn rebuild_jobs(&self, document: &mut Document) -> Vec<RebuildJob> {
        self.benches()
            .flat_map(|wb| wb.rebuild_jobs(document))
            .collect()
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
                let geometry = self
                    .owner_of(&node.workbench_id)?
                    .passive_geometry(document, *id, node)?;
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
