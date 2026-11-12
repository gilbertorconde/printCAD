//! Variable sets: named objects in the document, each a list of variables
//! defined by formulas (`nozzle = 0.4 mm`, `wall = 3 * Printer.nozzle`).
//!
//! A set is a feature node with no body, of kind [`VARIABLES_KIND`], named
//! as the set is (`Printer`); formulas anywhere read its variables as
//! `Printer.nozzle`. Adding, editing and removing them are ordinary
//! feature edits, so undo, saving and peers need nothing new.

use serde::{Deserialize, Serialize};

use crate::WorkbenchId;
use crate::feature::{FeatureId, WorkbenchFeature};

/// The feature kind of a variable set.
pub const VARIABLES_KIND: &str = "core.variables";

/// How [`crate::Document::new_object_name`] numbers a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Numbering {
    /// `base` while no object has it, then `base` and this separator
    /// before 2, 3, ...
    AfterBare(&'static str),
    /// `base 1`, `base 2`, ...: always numbered, after a space.
    Counted,
}

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

/// A variable a field can read, from [`crate::Document::variables_of`].
#[derive(Debug, Clone, PartialEq)]
pub struct VariableChoice {
    pub set: FeatureId,
    pub name: String,
    /// As a formula reads it: `Printer.nozzle`.
    pub reference: String,
    /// What it last came to.
    pub value: crate::expr::Quantity,
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

    /// `base`, or `base_2`, `base_3`, ...: see [`Self::new_object_name`].
    pub fn unused_object_name(&self, base: &str) -> String {
        self.new_object_name(base, Numbering::AfterBare("_"))
    }

    /// A name for a new object made from `base`: numbered one past the
    /// highest number `base` carries among the document's objects, so no
    /// two objects share it and a name never comes back while an object
    /// with a later number holds on to it.
    pub fn new_object_name(&self, base: &str, numbering: Numbering) -> String {
        let sep = match numbering {
            Numbering::AfterBare(sep) => sep,
            Numbering::Counted => " ",
        };
        let highest = self
            .feature_tree()
            .all_nodes()
            .filter_map(|(_, n)| {
                if n.name == base {
                    return Some(1);
                }
                n.name
                    .strip_prefix(base)?
                    .strip_prefix(sep)?
                    .parse::<u32>()
                    .ok()
            })
            .max();
        match (numbering, highest) {
            (Numbering::AfterBare(_), None) => base.to_string(),
            (_, highest) => format!("{base}{sep}{}", highest.unwrap_or(0) + 1),
        }
    }

    /// `base`, or `base_2`, `base_3`, ...: the first variable `set` does
    /// not have (with no set, `base`).
    pub fn unused_variable_name(&self, set: Option<FeatureId>, base: &str) -> String {
        let taken: Vec<String> = set
            .and_then(|set| self.variable_set(set).ok())
            .map(|data| data.variables.into_iter().map(|v| v.name).collect())
            .unwrap_or_default();
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_string()
                } else {
                    format!("{base}_{n}")
                }
            })
            .find(|name| !taken.contains(name))
            .unwrap_or_else(|| base.to_string())
    }

    /// Every variable whose last value is a `dim` quantity, set by set in
    /// the order they were made: what a field holding `dim` can read
    /// as it stands.
    pub fn variables_of(&self, dim: crate::expr::Dim) -> Vec<VariableChoice> {
        let mut out = Vec::new();
        for (set, set_name, data) in self.variable_sets() {
            let slots = self.evaluated_slots(set);
            for variable in data.variables {
                let Some(Ok(value)) = slots
                    .iter()
                    .find(|s| s.name.as_deref() == Some(variable.name.as_str()))
                    .map(|s| s.result.clone())
                else {
                    continue;
                };
                if value.dim != dim {
                    continue;
                }
                out.push(VariableChoice {
                    reference: format!(
                        "{}.{}",
                        crate::expr::quote_name(&set_name),
                        crate::expr::quote_name(&variable.name)
                    ),
                    set,
                    name: variable.name,
                    value,
                });
            }
        }
        out
    }

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
    /// the new text of one it changes: the variables of every set, the
    /// formulas of every feature and the configurations table's cells. Each
    /// change is an ordinary edit.
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
        let mut table_update = None;
        if let Some((id, mut table)) = self.configurations() {
            let mut changed = false;
            for text in table
                .columns
                .iter_mut()
                .chain(table.rows.iter_mut().flat_map(|r| r.values.iter_mut()))
            {
                if let Some(new) = rewrite(text) {
                    *text = new;
                    changed = true;
                }
            }
            if changed {
                table_update = Some((id, table.to_json()));
            }
        }
        for (id, data) in set_updates.into_iter().chain(table_update) {
            if let Err(why) = self.update_feature_data(id, data) {
                tracing::warn!("a renamed reference in {id:?} was not rewritten: {why}");
            }
        }
        for (id, key, text) in formula_updates {
            if let Err(why) = self.set_feature_formula(id, key.clone(), Some(text)) {
                tracing::warn!("the formula {key} of {id:?} was not rewritten: {why}");
            }
        }
    }
}

impl crate::Document {
    /// What `text` comes to against the values the document's formulas
    /// last came to: for a field holding `want`, or as it is. For the
    /// interface, which has no registry to evaluate with.
    pub fn evaluate_formula(
        &self,
        text: &str,
        want: Option<crate::expr::Dim>,
    ) -> Result<crate::expr::Quantity, String> {
        let ctx = crate::expr::Context {
            length_unit: self.display_unit(),
            resolve: &LastValues(self),
        };
        match want {
            Some(dim) => crate::expr::evaluate_as(text, dim, &ctx)
                .map(|v| crate::expr::Quantity::new(v, dim)),
            None => crate::expr::evaluate(text, &ctx),
        }
        .map_err(|e| e.message)
    }

    /// Every `Object.property` a formula can read, written as a formula
    /// writes it, for completion.
    pub fn formula_references(&self) -> Vec<String> {
        let mut out: Vec<(u64, String)> = Vec::new();
        for (id, node) in self.feature_tree().all_nodes() {
            for slot in self.evaluated_slots(*id) {
                if let Some(name) = &slot.name {
                    out.push((
                        node.seq,
                        format!(
                            "{}.{}",
                            crate::expr::quote_name(&node.name),
                            crate::expr::quote_name(name)
                        ),
                    ));
                }
            }
        }
        out.sort();
        out.into_iter().map(|(_, r)| r).collect()
    }
}

/// References read from the last evaluation.
struct LastValues<'a>(&'a crate::Document);

impl crate::expr::Resolve for LastValues<'_> {
    fn resolve(&self, object: &str, property: &str) -> Result<crate::expr::Quantity, String> {
        let id = self
            .0
            .object_named(object)
            .ok_or_else(|| format!("nothing is called {}", crate::expr::quote_name(object)))?;
        self.0
            .evaluated_slots(id)
            .iter()
            .find(|s| s.name.as_deref() == Some(property))
            .ok_or_else(|| {
                format!(
                    "{} has no {}",
                    crate::expr::quote_name(object),
                    crate::expr::quote_name(property)
                )
            })?
            .result
            .clone()
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

    /// A new name is never one an object has: numbered past the highest
    /// number in use, so a gap a deletion left is not filled while a
    /// later number is held.
    #[test]
    fn a_new_object_name_is_numbered_past_every_one_in_use() {
        use super::Numbering::{AfterBare, Counted};
        let mut doc = Document::new("t");
        assert_eq!(doc.new_object_name("Pad", AfterBare("_")), "Pad");
        assert_eq!(doc.new_object_name("Fit", Counted), "Fit 1");
        for name in ["Pad", "Padding", "Pad_3", "Fit 1", "Fit 2"] {
            doc.add_variable_set(name).unwrap();
        }
        assert_eq!(doc.unused_object_name("Pad"), "Pad_4");
        assert_eq!(doc.new_object_name("Pad", AfterBare(" ")), "Pad 2");
        let first = doc.object_named("Fit 1").unwrap();
        doc.remove_feature(first).unwrap();
        assert_eq!(doc.new_object_name("Fit", Counted), "Fit 3");
    }

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
