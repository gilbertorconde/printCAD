//! Configurations: named rows of values for chosen variables (a small,
//! medium and large version of a part), one of them active.
//!
//! The table is one feature node with no body, of kind
//! [`CONFIGURATIONS_KIND`]. Its columns are variables (`Size.width`); a row
//! gives each a formula, or leaves it empty to keep the variable's own.
//! While a row is active its formulas stand in for those variables' own
//! when the document is evaluated, so every number that follows them
//! follows the row. Switching rows is an ordinary edit, undone like any.

use serde::{Deserialize, Serialize};

use crate::expr;
use crate::feature::{FeatureId, WorkbenchFeature};
use crate::variables::VARIABLES_KIND;
use crate::{DocumentResult, WorkbenchId};

/// The feature kind of the configurations table.
pub const CONFIGURATIONS_KIND: &str = "core.configurations";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Configurations {
    /// The variables the rows set, as formulas read them (`Size.width`).
    pub columns: Vec<String>,
    pub rows: Vec<Configuration>,
    /// The row in effect, by name; `None` leaves every variable its own.
    pub active: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub name: String,
    /// A formula per column; an empty one keeps the variable's own.
    pub values: Vec<String>,
}

impl Configurations {
    /// The formulas the active row puts in place of variables' own, as
    /// `((set, variable), formula)`.
    pub fn overrides(&self) -> Vec<((String, String), String)> {
        let Some(row) = self
            .active
            .as_ref()
            .and_then(|name| self.rows.iter().find(|r| &r.name == name))
        else {
            return Vec::new();
        };
        self.columns
            .iter()
            .zip(&row.values)
            .filter(|(_, value)| !value.trim().is_empty())
            .filter_map(|(column, value)| {
                let reference = column_reference(column)?;
                Some((reference, value.clone()))
            })
            .collect()
    }

    fn row_mut(&mut self, name: &str) -> Result<&mut Configuration, String> {
        self.rows
            .iter_mut()
            .find(|r| r.name == name)
            .ok_or_else(|| format!("no configuration is called {name}"))
    }

    fn column(&self, variable: &str) -> Result<usize, String> {
        let wanted = column_reference(variable).ok_or_else(|| not_a_variable(variable))?;
        self.columns
            .iter()
            .position(|c| column_reference(c).as_ref() == Some(&wanted))
            .ok_or_else(|| format!("{variable} is not a column of the configurations"))
    }
}

/// The `(object, property)` a column names.
fn column_reference(column: &str) -> Option<(String, String)> {
    let refs = expr::references(column).ok()?;
    match refs.as_slice() {
        [r] => Some((r.object.clone(), r.property.clone())),
        _ => None,
    }
}

fn not_a_variable(text: &str) -> String {
    format!("{text} does not name a variable: write it as Set.name")
}

impl WorkbenchFeature for Configurations {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(CONFIGURATIONS_KIND)
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
        "Configurations"
    }
}

impl crate::Document {
    /// The configurations table, if the document has one.
    pub fn configurations(&self) -> Option<(FeatureId, Configurations)> {
        self.feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == CONFIGURATIONS_KIND)
            .min_by_key(|(_, n)| n.seq)
            .and_then(|(id, n)| Some((*id, Configurations::from_json(&n.data).ok()?)))
    }

    /// Edit the table with `change`, making it first when there is none.
    fn edit_configurations(
        &mut self,
        change: impl FnOnce(&mut Configurations) -> Result<(), String>,
    ) -> Result<(), String> {
        let (id, mut table) = match self.configurations() {
            Some(found) => found,
            None => {
                let name = (1..)
                    .map(|n| match n {
                        1 => "Configurations".to_string(),
                        n => format!("Configurations {n}"),
                    })
                    .find(|name| !self.has_object_named(name))
                    .expect("a free name");
                let id = self
                    .add_feature(Configurations::default(), name)
                    .map_err(|e| e.to_string())?;
                (id, Configurations::default())
            }
        };
        change(&mut table)?;
        self.update_feature_data(id, table.to_json())
            .map_err(|e| e.to_string())
    }

    /// Add configuration `name`; its values start empty, or as `like`'s.
    pub fn add_configuration(&mut self, name: &str, like: Option<&str>) -> Result<(), String> {
        if !expr::is_valid_name(name) {
            return Err(format!("`{name}` cannot be a name"));
        }
        self.edit_configurations(|t| {
            if t.rows.iter().any(|r| r.name == name) {
                return Err(format!("a configuration is already called {name}"));
            }
            let values = match like {
                Some(like) => t.row_mut(like)?.values.clone(),
                None => vec![String::new(); t.columns.len()],
            };
            t.rows.push(Configuration {
                name: name.to_string(),
                values,
            });
            Ok(())
        })
    }

    pub fn remove_configuration(&mut self, name: &str) -> Result<(), String> {
        self.edit_configurations(|t| {
            let before = t.rows.len();
            t.rows.retain(|r| r.name != name);
            if t.rows.len() == before {
                return Err(format!("no configuration is called {name}"));
            }
            if t.active.as_deref() == Some(name) {
                t.active = None;
            }
            Ok(())
        })
    }

    pub fn rename_configuration(&mut self, name: &str, to: &str) -> Result<(), String> {
        if !expr::is_valid_name(to) {
            return Err(format!("`{to}` cannot be a name"));
        }
        self.edit_configurations(|t| {
            if t.rows.iter().any(|r| r.name == to) {
                return Err(format!("a configuration is already called {to}"));
            }
            t.row_mut(name)?.name = to.to_string();
            if t.active.as_deref() == Some(name) {
                t.active = Some(to.to_string());
            }
            Ok(())
        })
    }

    /// Make `variable` (`Set.name`) a column: each row may set it.
    pub fn add_configuration_column(&mut self, variable: &str) -> Result<(), String> {
        let (set, name) = column_reference(variable).ok_or_else(|| not_a_variable(variable))?;
        let is_variable = self.object_named(&set).is_some_and(|id| {
            self.get_feature_meta(id).is_some_and(|n| {
                n.workbench_id.as_str() == VARIABLES_KIND
                    && crate::VariableSet::from_json(&n.data)
                        .is_ok_and(|s| s.variable(&name).is_some())
            })
        });
        if !is_variable {
            return Err(format!("{variable} is not a variable of a variable set"));
        }
        self.edit_configurations(|t| {
            if t.column(variable).is_ok() {
                return Err(format!("{variable} is already a column"));
            }
            t.columns.push(variable.trim().to_string());
            for row in &mut t.rows {
                row.values.push(String::new());
            }
            Ok(())
        })
    }

    pub fn remove_configuration_column(&mut self, variable: &str) -> Result<(), String> {
        self.edit_configurations(|t| {
            let i = t.column(variable)?;
            t.columns.remove(i);
            for row in &mut t.rows {
                if i < row.values.len() {
                    row.values.remove(i);
                }
            }
            Ok(())
        })
    }

    /// What configuration `name` gives `variable`: a formula, or empty to
    /// keep the variable's own.
    pub fn set_configuration_value(
        &mut self,
        name: &str,
        variable: &str,
        value: &str,
    ) -> Result<(), String> {
        if !value.trim().is_empty() {
            expr::check_syntax(value).map_err(|e| e.message)?;
        }
        self.edit_configurations(|t| {
            let i = t.column(variable)?;
            let row = t.row_mut(name)?;
            if row.values.len() <= i {
                row.values.resize(i + 1, String::new());
            }
            row.values[i] = value.trim().to_string();
            Ok(())
        })
    }

    /// Put configuration `name` in effect, or none.
    pub fn activate_configuration(&mut self, name: Option<&str>) -> Result<(), String> {
        self.edit_configurations(|t| {
            if let Some(name) = name
                && !t.rows.iter().any(|r| r.name == name)
            {
                return Err(format!("no configuration is called {name}"));
            }
            t.active = name.map(str::to_string);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn a_table_is_made_on_first_use_and_checks_what_it_is_given() {
        let mut doc = Document::new("t");
        let size = doc.add_variable_set("Size").unwrap();
        doc.set_variable(size, "width", "40 mm", None).unwrap();
        assert!(doc.configurations().is_none());
        doc.add_configuration("S", None).unwrap();
        assert!(doc.add_configuration("S", None).is_err());
        assert!(doc.add_configuration_column("Size.nope").is_err());
        assert!(doc.add_configuration_column("width").is_err());
        doc.add_configuration_column("Size.width").unwrap();
        doc.set_configuration_value("S", "Size.width", "30 mm")
            .unwrap();
        doc.add_configuration("L", Some("S")).unwrap();
        doc.set_configuration_value("L", "Size.width", "60 mm")
            .unwrap();
        assert!(
            doc.set_configuration_value("L", "Size.width", "60 +")
                .is_err()
        );
        doc.activate_configuration(Some("L")).unwrap();
        let (_, table) = doc.configurations().unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(
            table.overrides(),
            [(
                ("Size".to_string(), "width".to_string()),
                "60 mm".to_string()
            )]
        );
        doc.rename_configuration("L", "Large").unwrap();
        assert_eq!(
            doc.configurations().unwrap().1.active.as_deref(),
            Some("Large")
        );
        doc.remove_configuration("Large").unwrap();
        assert_eq!(doc.configurations().unwrap().1.active, None);
    }
}

#[cfg(test)]
mod evaluation {
    use crate::Document;
    use crate::expr::Quantity;

    #[test]
    fn the_active_row_sets_its_variables_and_renames_reach_the_table() {
        let mut doc = Document::new("t");
        let size = doc.add_variable_set("Size").unwrap();
        doc.set_variable(size, "width", "40 mm", None).unwrap();
        doc.set_variable(size, "half", "Size.width / 2", None)
            .unwrap();
        doc.add_configuration("S", None).unwrap();
        doc.add_configuration("L", None).unwrap();
        doc.add_configuration_column("Size.width").unwrap();
        doc.set_configuration_value("S", "Size.width", "30 mm")
            .unwrap();
        doc.set_configuration_value("L", "Size.width", "Size.base * 3")
            .unwrap();
        doc.set_variable(size, "base", "20 mm", None).unwrap();
        let half = |doc: &Document| {
            let e = crate::evaluate::evaluate_document(doc, &|_| Vec::new());
            e.slots[&size]
                .iter()
                .find(|s| s.key == "half")
                .unwrap()
                .result
                .clone()
        };
        assert_eq!(half(&doc), Ok(Quantity::length(20.0)), "no row: its own");
        doc.activate_configuration(Some("S")).unwrap();
        assert_eq!(half(&doc), Ok(Quantity::length(15.0)));
        doc.activate_configuration(Some("L")).unwrap();
        assert_eq!(half(&doc), Ok(Quantity::length(30.0)));

        doc.rename_variable(size, "width", "w").unwrap();
        doc.rename_variable(size, "base", "b").unwrap();
        let (_, table) = doc.configurations().unwrap();
        assert_eq!(table.columns, ["Size.w"]);
        assert_eq!(table.rows[1].values, ["Size.b * 3"]);
        assert_eq!(half(&doc), Ok(Quantity::length(30.0)), "still the row's");
    }
}
