//! Workbench registry used by the application shell.

use std::collections::HashMap;

use crate::feature::FeatureNode;
use crate::workbench::{
    CommandDescriptor, FeatureInfo, ToolDescriptor, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchId,
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

    pub fn tools_for(&self, id: &WorkbenchId) -> DocumentResult<&[ToolDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.tools())
    }

    pub fn commands_for(&self, id: &WorkbenchId) -> DocumentResult<&[CommandDescriptor]> {
        let entry = self
            .workbenches
            .get(id.as_str())
            .ok_or_else(|| DocumentError::WorkbenchMissing(id.as_str().to_owned()))?;
        Ok(entry.context.commands())
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
