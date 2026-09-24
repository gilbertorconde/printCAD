//! Variable sets: named objects in the document, each a list of variables
//! defined by formulas (`nozzle = 0.4 mm`, `wall = 3 * Printer.nozzle`).
//!
//! A set is a feature node with no body, of kind [`VARIABLES_KIND`], named
//! as the set is (`Printer`); formulas anywhere read its variables as
//! `Printer.nozzle`. Adding, editing and removing them are ordinary
//! feature edits, so undo, saving and peers need nothing new.

use serde::{Deserialize, Serialize};

use crate::feature::{FeatureId, WorkbenchFeature};
use crate::{DocumentResult, WorkbenchId};

/// The feature kind of a variable set.
pub const VARIABLES_KIND: &str = "core.variables";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VariableSet {
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Variable {
    pub name: String,
    /// What it is, as typed: `0.4 mm`, `3 * Printer.nozzle`.
    pub formula: String,
    pub comment: String,
}

impl VariableSet {
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }
}

impl WorkbenchFeature for VariableSet {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(VARIABLES_KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        Ok(serde_json::from_value(value.clone())?)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Variables"
    }
}
