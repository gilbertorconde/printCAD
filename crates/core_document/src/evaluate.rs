//! Working out every formula in a document.
//!
//! The slots are the variables of every variable set and the numeric
//! properties each bench lists for its features ([`Parameter`], found in
//! the feature's JSON by pointer, so this needs no bench's types). Each
//! slot is worked out once, the ones it reads first; a slot that comes
//! back to itself is a loop, reported on every slot in it. The result is
//! derived state: each feature with a formula gets a copy of its data with
//! the values in (`Document::feature_values`), and a feature whose values
//! came out different from last time is marked for rebuilding.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::Value;

use crate::expr::{self, Context, Dim, Quantity, Resolve};
use crate::feature::{FeatureId, FeatureNode};
use crate::param;
use crate::units::Unit;
use crate::variables::{VARIABLES_KIND, VariableSet};
use crate::{Document, WorkbenchFeature};

/// A number of a feature that a formula may set, and that formulas may
/// read when it has a name.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// What formulas call it (`length`, a named sketch dimension); `None`
    /// when formulas cannot refer to it.
    pub name: Option<String>,
    /// What the UI calls it.
    pub label: String,
    pub dim: Dim,
    /// Where it is in the feature's JSON (a JSON pointer: `/Pad/length`).
    pub pointer: String,
    /// What the JSON holds for one unit of formula value (millimetre or
    /// degree): 1, or π/180 for an angle kept in radians.
    pub scale: f64,
}

impl Parameter {
    pub fn new(name: &str, label: &str, dim: Dim, pointer: impl Into<String>) -> Self {
        Self {
            name: Some(name.to_string()),
            label: label.to_string(),
            dim,
            pointer: pointer.into(),
            scale: 1.0,
        }
    }

    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    pub fn unnamed(mut self) -> Self {
        self.name = None;
        self
    }
}

/// What one slot came to.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotValue {
    pub name: Option<String>,
    pub label: String,
    /// Its formula, when it has one.
    pub formula: Option<String>,
    pub result: Result<Quantity, String>,
}

/// Every formula of a document, worked out.
#[derive(Debug, Clone, Default)]
pub struct Evaluation {
    /// Each feature with a formula: its data with the values in.
    pub data: HashMap<FeatureId, Value>,
    /// Every slot of every feature that has one, in order.
    pub slots: HashMap<FeatureId, Vec<SlotValue>>,
}

enum Source {
    Literal(f64),
    Formula(String),
}

struct Slot {
    feature: FeatureId,
    name: Option<String>,
    label: String,
    /// `None` for a variable: it is whatever its formula gives.
    dim: Option<Dim>,
    source: Source,
    pointer: Option<String>,
    scale: f64,
}

#[derive(Clone)]
enum State {
    Pending,
    Busy,
    Done(Result<Quantity, String>),
}

struct Graph {
    slots: Vec<Slot>,
    /// Objects by name: more than one under a name makes it ambiguous.
    objects: HashMap<String, Vec<FeatureId>>,
    index: HashMap<(FeatureId, String), usize>,
    state: RefCell<Vec<State>>,
    unit: Unit,
}

impl Graph {
    fn value(&self, i: usize) -> Result<Quantity, String> {
        match &self.state.borrow()[i] {
            State::Done(result) => return result.clone(),
            State::Busy => return Err(LOOP.to_string()),
            State::Pending => {}
        }
        self.state.borrow_mut()[i] = State::Busy;
        let slot = &self.slots[i];
        let result = match &slot.source {
            Source::Literal(v) => Ok(Quantity::new(v / slot.scale, slot.dim.unwrap_or_default())),
            Source::Formula(text) => {
                let ctx = Context {
                    length_unit: self.unit,
                    resolve: self,
                };
                match slot.dim {
                    Some(dim) => expr::evaluate_as(text, dim, &ctx).map(|v| Quantity::new(v, dim)),
                    None => expr::evaluate(text, &ctx),
                }
                .map_err(|e| e.message)
            }
        };
        self.state.borrow_mut()[i] = State::Done(result.clone());
        result
    }
}

/// What a slot says when it reads itself, however far round.
const LOOP: &str = "a loop: this depends on itself";

impl Resolve for Graph {
    fn resolve(&self, object: &str, property: &str) -> Result<Quantity, String> {
        let reference = format!(
            "{}.{}",
            expr::quote_name(object),
            expr::quote_name(property)
        );
        let ids = self
            .objects
            .get(object)
            .ok_or_else(|| format!("nothing is called {}", expr::quote_name(object)))?;
        if ids.len() > 1 {
            return Err(format!(
                "{} objects are called {}: rename all but one",
                ids.len(),
                expr::quote_name(object)
            ));
        }
        let i = *self
            .index
            .get(&(ids[0], property.to_string()))
            .ok_or_else(|| {
                format!(
                    "{} has no {}",
                    expr::quote_name(object),
                    expr::quote_name(property)
                )
            })?;
        self.value(i).map_err(|why| {
            if why == LOOP {
                why
            } else {
                format!("{reference} has an error: {why}")
            }
        })
    }
}

/// Work out every formula of `document`; `parameters` lists each
/// feature's numeric properties (the registry asks the owning bench).
pub fn evaluate_document(
    document: &Document,
    parameters: &dyn Fn(&FeatureNode) -> Vec<Parameter>,
) -> Evaluation {
    let mut slots = Vec::new();
    let mut objects: HashMap<String, Vec<FeatureId>> = HashMap::new();
    let mut nodes: Vec<(&FeatureId, &FeatureNode)> = document.feature_tree().all_nodes().collect();
    nodes.sort_by_key(|(_, n)| n.seq);
    for (id, node) in nodes {
        objects.entry(node.name.clone()).or_default().push(*id);
        if node.workbench_id.as_str() == VARIABLES_KIND {
            let Ok(set) = VariableSet::from_json(&node.data) else {
                continue;
            };
            for variable in set.variables {
                slots.push(Slot {
                    feature: *id,
                    name: Some(variable.name.clone()),
                    label: variable.name,
                    dim: None,
                    source: Source::Formula(variable.formula),
                    pointer: None,
                    scale: 1.0,
                });
            }
            continue;
        }
        for p in parameters(node) {
            let Some((value, formula)) = node.data.pointer(&p.pointer).and_then(param::read) else {
                continue;
            };
            slots.push(Slot {
                feature: *id,
                name: p.name,
                label: p.label,
                dim: Some(p.dim),
                source: match formula {
                    Some(text) => Source::Formula(text.to_string()),
                    None => Source::Literal(value),
                },
                pointer: Some(p.pointer),
                scale: p.scale,
            });
        }
    }
    let index = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some(((s.feature, s.name.clone()?), i)))
        .collect();
    let graph = Graph {
        state: RefCell::new(vec![State::Pending; slots.len()]),
        slots,
        objects,
        index,
        unit: document.display_unit(),
    };

    let mut out = Evaluation::default();
    for i in 0..graph.slots.len() {
        let result = graph.value(i);
        let slot = &graph.slots[i];
        let formula = match &slot.source {
            Source::Formula(text) => Some(text.clone()),
            Source::Literal(_) => None,
        };
        if let (Some(pointer), Some(_), Ok(q)) = (&slot.pointer, &formula, &result) {
            let data = out.data.entry(slot.feature).or_insert_with(|| {
                document
                    .get_feature_data(slot.feature)
                    .cloned()
                    .unwrap_or_default()
            });
            if let Some(target) = data.pointer_mut(pointer) {
                param::write_value(target, q.value * slot.scale);
            }
        }
        out.slots.entry(slot.feature).or_default().push(SlotValue {
            name: slot.name.clone(),
            label: slot.label.clone(),
            formula,
            result,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variables::Variable;
    use serde_json::json;

    /// A feature kind with a length at `/length` and an angle kept in
    /// radians at `/turn`.
    #[derive(Debug, Clone)]
    struct Block(Value);

    impl WorkbenchFeature for Block {
        fn workbench_id() -> crate::WorkbenchId {
            crate::WorkbenchId::from("test.block")
        }
        fn to_json(&self) -> Value {
            self.0.clone()
        }
        fn from_json(value: &Value) -> crate::DocumentResult<Self> {
            Ok(Self(value.clone()))
        }
        fn dependencies(&self) -> Vec<FeatureId> {
            Vec::new()
        }
        fn name(&self) -> &str {
            "Block"
        }
    }

    fn parameters(node: &FeatureNode) -> Vec<Parameter> {
        if node.workbench_id.as_str() != "test.block" {
            return Vec::new();
        }
        vec![
            Parameter::new("length", "Length", Dim::LENGTH, "/length"),
            Parameter::new("turn", "Turn", Dim::ANGLE, "/turn")
                .scaled(std::f64::consts::PI / 180.0),
        ]
    }

    fn set(document: &mut Document, name: &str, vars: &[(&str, &str)]) -> FeatureId {
        let set = VariableSet {
            variables: vars
                .iter()
                .map(|(n, f)| Variable {
                    name: n.to_string(),
                    formula: f.to_string(),
                    comment: String::new(),
                })
                .collect(),
        };
        document.add_feature(set, name.to_string()).unwrap()
    }

    fn block(document: &mut Document, name: &str, data: Value) -> FeatureId {
        document.add_feature(Block(data), name.to_string()).unwrap()
    }

    fn slot<'a>(e: &'a Evaluation, id: FeatureId, name: &str) -> &'a SlotValue {
        e.slots[&id]
            .iter()
            .find(|s| s.name.as_deref() == Some(name))
            .unwrap()
    }

    #[test]
    fn formulas_read_variables_and_other_features_and_fill_in_the_data() {
        let mut doc = Document::new("t");
        let printer = set(
            &mut doc,
            "Printer",
            &[
                ("nozzle", "0.4 mm"),
                ("wall", "3 * Printer.nozzle"),
                ("walls", "3"),
            ],
        );
        let base = block(&mut doc, "Base", json!({"length": 10.0, "turn": 0.0}));
        let top = block(
            &mut doc,
            "Top plate",
            json!({
                "length": {"formula": "Base.length + Printer.wall", "value": 0.0},
                "turn": {"formula": "atan2(Printer.nozzle, Printer.nozzle)", "value": 0.0},
            }),
        );
        let e = evaluate_document(&doc, &parameters);
        assert_eq!(
            slot(&e, printer, "wall").result,
            Ok(Quantity::length(1.2000000000000002))
        );
        assert_eq!(slot(&e, printer, "walls").result, Ok(Quantity::number(3.0)));
        let data = &e.data[&top];
        assert!((data["length"]["value"].as_f64().unwrap() - 11.2).abs() < 1e-9);
        assert_eq!(data["length"]["formula"], "Base.length + Printer.wall");
        assert!(
            (data["turn"]["value"].as_f64().unwrap() - std::f64::consts::FRAC_PI_4).abs() < 1e-9
        );
        assert!(!e.data.contains_key(&base), "no formula, no copy");
        // Its degrees read back from radians.
        let e2 = {
            let mut doc2 = doc.clone();
            block(
                &mut doc2,
                "Probe",
                json!({"length": {"formula": "`Top plate`.turn * 1 mm / 1 deg", "value": 0}, "turn": 0}),
            );
            evaluate_document(&doc2, &parameters)
        };
        let probe = e2
            .slots
            .iter()
            .find(|(_, s)| {
                s.iter()
                    .any(|v| v.formula.as_deref().is_some_and(|f| f.starts_with("`Top")))
            })
            .map(|(id, _)| *id)
            .unwrap();
        assert!((slot(&e2, probe, "length").result.clone().unwrap().value - 45.0).abs() < 1e-9);
    }

    #[test]
    fn loops_unknown_names_and_wrong_kinds_are_reported_where_they_are() {
        let mut doc = Document::new("t");
        let s = set(
            &mut doc,
            "S",
            &[
                ("a", "S.b + 1 mm"),
                ("b", "S.a"),
                ("c", "S.nope"),
                ("d", "Other.x"),
                ("e", "S.c * 2"),
            ],
        );
        let blk = block(
            &mut doc,
            "B",
            json!({"length": {"formula": "30 deg", "value": 5.0}, "turn": 0}),
        );
        let e = evaluate_document(&doc, &parameters);
        let err = |name: &str| slot(&e, s, name).result.clone().unwrap_err();
        assert!(err("a").contains("loop"), "{}", err("a"));
        assert!(err("b").contains("loop"), "{}", err("b"));
        assert_eq!(err("c"), "S has no nope");
        assert_eq!(err("d"), "nothing is called Other");
        assert!(err("e").starts_with("S.c has an error"), "{}", err("e"));
        let wrong = slot(&e, blk, "length").result.clone().unwrap_err();
        assert!(wrong.contains("an angle where a length"), "{wrong}");
        assert!(
            !e.data.contains_key(&blk),
            "a failed formula leaves the stored value"
        );
    }

    #[test]
    fn two_objects_with_one_name_are_ambiguous() {
        let mut doc = Document::new("t");
        set(&mut doc, "Dup", &[("x", "1 mm")]);
        set(&mut doc, "Dup", &[("x", "2 mm")]);
        let b = block(
            &mut doc,
            "B",
            json!({"length": {"formula": "Dup.x", "value": 0}, "turn": 0}),
        );
        let e = evaluate_document(&doc, &parameters);
        let why = slot(&e, b, "length").result.clone().unwrap_err();
        assert!(why.contains("2 objects are called Dup"), "{why}");
    }

    fn settle(doc: &mut Document) -> Vec<FeatureId> {
        let e = evaluate_document(doc, &parameters);
        let marked = doc.apply_evaluation(e);
        for (id, _) in doc
            .feature_tree()
            .all_nodes()
            .map(|(i, n)| (*i, n.dirty))
            .collect::<Vec<_>>()
        {
            doc.clear_feature_dirty(id);
        }
        marked
    }

    #[test]
    fn a_changed_variable_marks_only_what_reads_it_and_leaves_the_document_unedited() {
        let mut doc = Document::new("t");
        let s = set(&mut doc, "S", &[("a", "2 mm"), ("b", "5 mm")]);
        let reads_a = block(
            &mut doc,
            "A",
            json!({"length": {"formula": "S.a * 2", "value": 0}, "turn": 0}),
        );
        let reads_b = block(
            &mut doc,
            "B",
            json!({"length": {"formula": "S.b", "value": 0}, "turn": 0}),
        );
        let plain = block(&mut doc, "C", json!({"length": 3, "turn": 0}));
        let first = settle(&mut doc);
        assert!(first.contains(&reads_a) && first.contains(&reads_b));
        assert!(!first.contains(&plain));
        assert_eq!(doc.feature_values(reads_a).unwrap()["length"]["value"], 4.0);
        assert_eq!(
            doc.feature_values(plain).unwrap()["length"],
            3.0,
            "raw data"
        );
        assert!(!doc.needs_evaluation());

        // Change `a`: only A moves.
        let mut data: VariableSet =
            VariableSet::from_json(doc.get_feature_data(s).unwrap()).unwrap();
        data.variables[0].formula = "3 mm".into();
        doc.update_feature_data(s, data.to_json()).unwrap();
        doc.mark_clean();
        assert!(doc.needs_evaluation());
        assert_eq!(settle(&mut doc), [reads_a]);
        assert_eq!(doc.feature_values(reads_a).unwrap()["length"]["value"], 6.0);
        assert!(!doc.metadata().dirty(), "evaluating is not an edit");
        assert_eq!(settle(&mut doc), [], "nothing moved since");

        // A snapshot carries what was worked out.
        let copy = doc.clone();
        assert_eq!(
            copy.feature_values(reads_a).unwrap()["length"]["value"],
            6.0
        );
        assert!(!copy.needs_evaluation());
    }

    #[test]
    fn a_bare_number_in_a_length_takes_the_documents_unit() {
        let mut doc = Document::new("t");
        doc.set_display_unit(Unit::In);
        let b = block(
            &mut doc,
            "B",
            json!({"length": {"formula": "2", "value": 0}, "turn": 0}),
        );
        let e = evaluate_document(&doc, &parameters);
        assert_eq!(slot(&e, b, "length").result, Ok(Quantity::length(50.8)));
    }
}
