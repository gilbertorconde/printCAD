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

impl crate::Document {
    /// The variable sets, in the order they were made.
    pub fn variable_sets(&self) -> Vec<(FeatureId, String, VariableSet)> {
        let mut sets: Vec<(u64, FeatureId, String, VariableSet)> = self
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == VARIABLES_KIND)
            .filter_map(|(id, n)| {
                Some((
                    n.seq,
                    *id,
                    n.name.clone(),
                    VariableSet::from_json(&n.data).ok()?,
                ))
            })
            .collect();
        sets.sort_by_key(|(seq, ..)| *seq);
        sets.into_iter()
            .map(|(_, id, name, set)| (id, name, set))
            .collect()
    }

    /// The object formulas call `name`, when exactly one is.
    pub fn object_named(&self, name: &str) -> Option<FeatureId> {
        let mut found = self
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.name == name)
            .map(|(id, _)| *id);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Whether some object is called `name`.
    pub fn has_object_named(&self, name: &str) -> bool {
        self.feature_tree().all_nodes().any(|(_, n)| n.name == name)
    }

    /// Make an empty variable set called `name`.
    pub fn add_variable_set(&mut self, name: &str) -> Result<FeatureId, String> {
        check_name(name)?;
        if self.has_object_named(name) {
            return Err(format!("something is already called {name}"));
        }
        self.add_feature(VariableSet::default(), name.to_string())
            .map_err(|e| e.to_string())
    }

    fn variable_set(&self, set: FeatureId) -> Result<VariableSet, String> {
        let node = self
            .get_feature_meta(set)
            .filter(|n| n.workbench_id.as_str() == VARIABLES_KIND)
            .ok_or("that is not a variable set")?;
        VariableSet::from_json(&node.data).map_err(|e| e.to_string())
    }

    /// Set variable `name` of `set` to `formula`, adding it when it is new;
    /// `comment` replaces its comment when given. The formula must parse;
    /// what it comes to is worked out with the rest.
    pub fn set_variable(
        &mut self,
        set: FeatureId,
        name: &str,
        formula: &str,
        comment: Option<&str>,
    ) -> Result<(), String> {
        check_name(name)?;
        crate::expr::check_syntax(formula).map_err(|e| e.message)?;
        let mut data = self.variable_set(set)?;
        match data.variables.iter_mut().find(|v| v.name == name) {
            Some(v) => {
                v.formula = formula.to_string();
                if let Some(comment) = comment {
                    v.comment = comment.to_string();
                }
            }
            None => data.variables.push(Variable {
                name: name.to_string(),
                formula: formula.to_string(),
                comment: comment.unwrap_or_default().to_string(),
            }),
        }
        self.update_feature_data(set, data.to_json())
            .map_err(|e| e.to_string())
    }

    /// Take variable `name` out of `set`. Formulas that read it report it
    /// missing.
    pub fn remove_variable(&mut self, set: FeatureId, name: &str) -> Result<(), String> {
        let mut data = self.variable_set(set)?;
        let before = data.variables.len();
        data.variables.retain(|v| v.name != name);
        if data.variables.len() == before {
            return Err(format!("the set has no {name}"));
        }
        self.update_feature_data(set, data.to_json())
            .map_err(|e| e.to_string())
    }

    /// Rename variable `old` of `set` to `new`, and every formula that
    /// reads it with it.
    pub fn rename_variable(&mut self, set: FeatureId, old: &str, new: &str) -> Result<(), String> {
        check_name(new)?;
        let mut data = self.variable_set(set)?;
        if data.variable(new).is_some() {
            return Err(format!("the set already has {new}"));
        }
        let variable = data
            .variables
            .iter_mut()
            .find(|v| v.name == old)
            .ok_or_else(|| format!("the set has no {old}"))?;
        variable.name = new.to_string();
        self.update_feature_data(set, data.to_json())
            .map_err(|e| e.to_string())?;
        let set_name = self
            .get_feature_meta(set)
            .map(|n| n.name.clone())
            .unwrap_or_default();
        self.rewrite_formulas(|text| crate::expr::rename_property(text, &set_name, old, new));
        Ok(())
    }

    /// Rewrite every formula in the document with `rewrite`, which answers
    /// the new text of one it changes: the variables of every set and the
    /// formulas of every feature. Each change is an ordinary edit.
    pub fn rewrite_formulas(&mut self, rewrite: impl Fn(&str) -> Option<String>) {
        let mut set_updates = Vec::new();
        let mut formula_updates = Vec::new();
        for (id, node) in self.feature_tree().all_nodes() {
            if node.workbench_id.as_str() == VARIABLES_KIND
                && let Ok(mut set) = VariableSet::from_json(&node.data)
            {
                let mut changed = false;
                for v in &mut set.variables {
                    if let Some(text) = rewrite(&v.formula) {
                        v.formula = text;
                        changed = true;
                    }
                }
                if changed {
                    set_updates.push((*id, set.to_json()));
                }
            }
            for (key, text) in &node.formulas {
                if let Some(text) = rewrite(text) {
                    formula_updates.push((*id, key.clone(), text));
                }
            }
        }
        for (id, data) in set_updates {
            let _ = self.update_feature_data(id, data);
        }
        for (id, key, text) in formula_updates {
            let _ = self.set_feature_formula(id, key, Some(text));
        }
    }
}

/// A name a variable or a set may have.
fn check_name(name: &str) -> Result<(), String> {
    if crate::expr::is_valid_name(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` cannot be a name: it is empty or has a backtick or a line break"
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn renaming_a_variable_or_its_set_rewrites_the_formulas_that_read_it() {
        let mut doc = Document::new("t");
        let printer = doc.add_variable_set("Printer").unwrap();
        doc.set_variable(printer, "nozzle", "0.4 mm", Some("the nozzle"))
            .unwrap();
        doc.set_variable(printer, "wall", "3 * Printer.nozzle", None)
            .unwrap();
        let other = doc.add_variable_set("Part").unwrap();
        doc.set_variable(other, "rib", "Printer.nozzle * 2", None)
            .unwrap();
        assert!(
            doc.add_variable_set("Printer").is_err(),
            "one name, one object"
        );
        assert!(doc.set_variable(printer, "bad", "2 +", None).is_err());

        doc.rename_variable(printer, "nozzle", "bore").unwrap();
        let sets = doc.variable_sets();
        assert_eq!(sets[0].2.variables[0].name, "bore");
        assert_eq!(sets[0].2.variables[0].comment, "the nozzle");
        assert_eq!(sets[0].2.variables[1].formula, "3 * Printer.bore");
        assert_eq!(sets[1].2.variables[0].formula, "Printer.bore * 2");

        doc.rename_feature(printer, "My printer");
        let sets = doc.variable_sets();
        assert_eq!(sets[0].2.variables[1].formula, "3 * `My printer`.bore");
        assert_eq!(sets[1].2.variables[0].formula, "`My printer`.bore * 2");
        assert_eq!(doc.object_named("My printer"), Some(printer));

        doc.remove_variable(other, "rib").unwrap();
        assert!(doc.remove_variable(other, "rib").is_err());
    }
}
